
//! OPDS 2.0 route handlers
//!
//! Everything under `/v2` answers in JSON.
//! The same library under `/` continues to return Atom
//!
//! however, `/{lib}/cover/{id}` and `/{lib}/file/{id}/{format}` are plain HTTP downloads

use actix_web::{web, HttpRequest, HttpResponse, Responder};
use rusqlite::Connection;
use serde::Serialize;
use serde_derive::Deserialize;
use std::sync::MutexGuard;

use crate::appstate::AppState;
use crate::authorized::Authorized;
use crate::calibre::{self, Book, Identifier};
use crate::config::CATALOG;
use crate::landing::{Kind, Section};
use crate::opds2::{
    BelongsTo, BookMetadata, Contributor, Feed, Group, Link, Publication, Series, Subject,
    ACQUISITION, BOOK, FEED, IMAGE, PUBLICATION, SEARCH, SORT_NEW,
};
use crate::routes::{origin, server_error};
use crate::shelf::{self, feed_of, page_url, window, Shelf, PER_PAGE};

#[derive(Deserialize)]
struct PageQuery {
    page: Option<usize>,
}

#[derive(Deserialize)]
struct SearchQuery {
    query: Option<String>,
    page: Option<usize>,
}

/// The connection of one library, or a 404 for a library Orca does not serve.
fn library<'a>(data: &'a AppState, lib: &str) -> Result<MutexGuard<'a, Connection>, HttpResponse> {
    match data.db.get(lib) {
        Some(db) => Ok(calibre::lock(db)),
        None => Err(HttpResponse::NotFound().body(format!("Database '{}' not found", lib))),
    }
}

fn feed(title: impl Into<String>, self_url: String, base: &str) -> Feed {
    Feed::new(title, self_url).link(Link::new(format!("{}/v2", base)).rel("start").mime(FEED))
}

/// A feed that stands inside one library also says how to search that library.
/// has to be `query`: apparently this is the one the ecosystem settled on
fn library_feed(title: impl Into<String>, self_url: String, base: &str, lib: &str) -> Feed {
    feed(title, self_url, base).link(
        Link::new(format!("{}/v2/{}/search?query={{query}}", base, lib))
            .rel(SEARCH)
            .mime(FEED)
            .templated(),
    )
}

fn json(value: &impl Serialize, mime: &str) -> HttpResponse {
    match serde_json::to_string(value) {
        Ok(body) => HttpResponse::Ok().content_type(mime).body(body),
        Err(e) => server_error("Error serialising feed", e),
    }
}

/// Atom can be found directly under `{base}/{lib}` (without the `/v2/`)
fn root_of(base: &str, lib: &str) -> String {
    format!("{}/v2/{}", base, lib)
}

fn sort_as(sort: &str, name: &str) -> Option<String> {
    match sort.trim() {
        "" => None,
        sort if sort == name => None,
        sort => Some(sort.to_string()),
    }
}

fn identifier(book: &Book, lib: &str) -> String {
    if book.uuid.is_empty() {
        format!("urn:orca:{}:book:{}", lib, book.id)
    } else {
        format!("urn:uuid:{}", book.uuid)
    }
}

/// One Calibre book as an OPDS 2.0 publication.
fn publication(book: &Book, lib: &str, base: &str) -> Publication {
    let mut links = vec![Link::new(format!("{}/v2/{}/book/{}", base, lib, book.id))
        .rel("self")
        .mime(PUBLICATION)];

    links.extend(book.formats.iter().map(|format| {
        Link::new(format!("{}/{}/file/{}/{}", base, lib, book.id, format.name))
            .rel(ACQUISITION)
            .mime(calibre::mime(&format.name))
            .title(format!("{}.{}", book.title, format.name))
    }));

    // This book as a web page: When the client let's you "open in browser"
    if let Some(page) = &book.alternate {
        links.push(Link::new(page.clone()).rel("alternate").mime("text/html"));
    }

    // Calibre keeps one cover and no thumbnail, so the one image is the cover.
    let images = match book.has_cover {
        true => vec![Link::new(format!("{}/{}/cover/{}", base, lib, book.id))
            .rel(IMAGE)
            .mime("image/jpeg")],
        false => Vec::new(),
    };

    Publication {
        metadata: BookMetadata {
            kind: BOOK,
            title: book.title.clone(),
            sort_as: sort_as(&book.sort, &book.title),
            identifier: Some(identifier(book, lib)),
            alt_identifier: book.identifiers.iter().filter_map(Identifier::uri).collect(),
            // link from a book to the rest of what its author wrote.
            author: book
                .authors
                .iter()
                .map(|author| Contributor {
                    name: author.name.clone(),
                    sort_as: sort_as(&author.sort, &author.name),
                    links: vec![
                        Link::new(format!("{}/v2/{}/authors/{}", base, lib, author.id)).mime(FEED),
                    ],
                })
                .collect(),
            language: book.languages.clone(),
            published: Some(book.pubdate.clone()),
            modified: Some(book.updated.clone()),
            description: match calibre::plain_text(&book.synopsis, calibre::UNWRAPPED).trim() {
                "" => None,
                synopsis => Some(synopsis.to_string()),
            },
            publisher: book
                .publisher
                .iter()
                .map(|publisher| Contributor {
                    name: publisher.name.clone(),
                    sort_as: None,
                    links: vec![
                        Link::new(format!("{}/v2/{}/publishers/{}", base, lib, publisher.id))
                            .mime(FEED),
                    ],
                })
                .collect(),
            subject: book
                .tags
                .iter()
                .map(|tag| Subject {
                    name: tag.name.clone(),
                    links: vec![Link::new(format!("{}/v2/{}/tags/{}", base, lib, tag.id)).mime(FEED)],
                })
                .collect(),
            belongs_to: book.series.as_ref().map(|series| BelongsTo {
                series: Series {
                    name: series.name.clone(),
                    sort_as: sort_as(&series.sort, &series.name),
                    identifier: Some(format!("urn:orca:{}:series:{}", lib, series.id)),
                    position: Some(series.index),
                    links: vec![
                        Link::new(format!("{}/v2/{}/series/{}", base, lib, series.id)).mime(FEED),
                    ],
                },
            }),
        },
        links,
        images,
    }
}

/// The catalog root: one navigation entry per library,
/// or a redirect when there is only one
#[actix_web::get("/v2")]
async fn catalog(data: web::Data<AppState>, auth: Authorized, req: HttpRequest) -> impl Responder {
    // Only what this client could open
    let mut libraries: Vec<&String> = data
        .db
        .keys()
        .filter(|lib| auth.may_reach(data.config, &format!("/v2/{}", lib)))
        .collect();
    libraries.sort();

    let base = origin(&req, data.config);

    if let [only] = libraries[..] {
        return HttpResponse::Found()
            .append_header(("Location", root_of(&base, only)))
            .finish();
    }

    // The whole catalog is as new as its newest library.
    let updated = data
        .db
        .values()
        .map(|db| calibre::updated(&calibre::lock(db)))
        .max();

    let navigation = libraries
        .iter()
        .map(|lib| {
            Link::new(format!("{}/v2/{}", base, lib))
                .rel("subsection")
                .mime(FEED)
                .title(lib.to_string())
        })
        .collect();

    // Left out rather than made up: `modified` has to be a date-time
    let mut root = feed(CATALOG, format!("{}/v2", base), &base).navigation(navigation);
    if let Some(updated) = updated {
        root = root.modified(updated);
    }

    json(&root, FEED)
}

#[actix_web::get("/v2/{lib}")]
async fn library_root(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match library(&data, &lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let counts = match calibre::counts(&db) {
        Ok(counts) => counts,
        Err(e) => return server_error("Error counting the library", e),
    };

    let base = origin(&req, data.config);
    let root = root_of(&base, &lib);

    let page = match data.landing(&lib).visible(&db, &counts) {
        Ok(page) => page,
        Err(e) => return server_error("Error reading the landing page", e),
    };

    // A section that shows books becomes a group, the rest stay links
    let mut navigation: Vec<Link> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();

    for section in &page {
        let href = page_url(&root, &section.feed(), 1);

        if section.inline() {
            let books = match section.preview(&db) {
                Ok(books) => books,
                Err(e) => return server_error("Error querying books", e),
            };

            let mut group = Group::new(section.title.clone(), href).publications(
                books.iter().map(|book| publication(book, &lib, &base)).collect(),
            );

            if let Some(total) = section.total(&counts) {
                group = group.count(total);
            }

            groups.push(group);
            continue;
        }

        let link = Link::new(href)
            .rel(match section.kind {
                Kind::New => SORT_NEW,
                _ => "subsection",
            })
            .mime(FEED)
            .title(section.title.clone());

        navigation.push(match section.total(&counts) {
            Some(total) => link.count(total),
            None => link,
        });
    }

    let root = library_feed(lib.clone(), format!("{}/v2/{}", base, lib), &base, &lib)
        .modified(calibre::updated(&db))
        .navigation(navigation)
        .groups(groups);

    json(&root, FEED)
}

/// One page of books, whichever shelf they come off.
fn books_feed(
    data: &AppState,
    req: &HttpRequest,
    lib: &str,
    shelf: Shelf,
    requested: usize,
) -> HttpResponse {
    let db = match library(data, lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let name = match shelf.name(&db) {
        Ok(name) => name,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return HttpResponse::NotFound().body(format!("Nothing shelved under {}", shelf.path()))
        }
        Err(e) => return server_error("Error querying shelf", e),
    };

    let total = match shelf.count(&db) {
        Ok(total) => total,
        Err(e) => return server_error("Error counting books", e),
    };
    let window = window(total, PER_PAGE, requested);

    let books = match shelf.books(&db, PER_PAGE, window.offset) {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    let base = origin(req, data.config);
    let root = root_of(&base, lib);
    let path = shelf.path();
    let mut page = library_feed(
        format!("{} | {}", lib, name),
        page_url(&root, &path, window.current),
        &base,
        lib,
    )
    .modified(calibre::updated(&db))
    .page(total, PER_PAGE, window.current);
    page = holding(page, &books, lib, &base);

    for link in shelf::page_links(&root, &path, &window) {
        page = page.link(Link::new(link.href).rel(link.rel).mime(FEED));
    }

    json(&page, FEED)
}

/// A feed has to hold one of `publications`, `navigation` or `groups`, so a
/// search that matched nothing cannot simply leave `publications` out.
fn holding(feed: Feed, books: &[Book], lib: &str, base: &str) -> Feed {
    match books.is_empty() {
        true => feed.navigation(vec![Link::new(format!("{}/v2/{}", base, lib))
            .rel("up")
            .mime(FEED)
            .title(format!("Back to {}", lib))]),
        false => {
            feed.publications(books.iter().map(|book| publication(book, lib, base)).collect())
        }
    }
}

/// Every book in the library, a page at a time.
#[actix_web::get("/v2/{lib}/books")]
async fn all_books(
    data: web::Data<AppState>,
    path: web::Path<String>,
    query: web::Query<PageQuery>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    books_feed(&data, &req, &lib, Shelf::Everything, query.page.unwrap_or(1))
}

/// Everything one author wrote.
#[actix_web::get("/v2/{lib}/authors/{id}")]
async fn books_by_author(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    query: web::Query<PageQuery>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, author) = path.into_inner();
    books_feed(&data, &req, &lib, Shelf::Author(author), query.page.unwrap_or(1))
}

/// Everything under one tag.
#[actix_web::get("/v2/{lib}/tags/{id}")]
async fn books_by_tag(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    query: web::Query<PageQuery>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, tag) = path.into_inner();
    books_feed(&data, &req, &lib, Shelf::Tag(tag), query.page.unwrap_or(1))
}

/// One series, in reading order.
#[actix_web::get("/v2/{lib}/series/{id}")]
async fn books_in_series(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    query: web::Query<PageQuery>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, series) = path.into_inner();
    books_feed(&data, &req, &lib, Shelf::Series(series), query.page.unwrap_or(1))
}

#[actix_web::get("/v2/{lib}/publishers/{id}")]
async fn books_by_publisher(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    query: web::Query<PageQuery>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, publisher) = path.into_inner();
    books_feed(&data, &req, &lib, Shelf::Publisher(publisher), query.page.unwrap_or(1))
}

/// Everything whose title or author matches `?query=`, paginated.
#[actix_web::get("/v2/{lib}/search")]
async fn search(
    data: web::Data<AppState>,
    path: web::Path<String>,
    asked: web::Query<SearchQuery>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let term = asked.query.clone().unwrap_or_default();
    books_feed(&data, &req, &lib, Shelf::Search(term), asked.page.unwrap_or(1))
}

/// One navigation entry per category, each leading to the shelf of its books.
/// `shelf` is the variant those entries lead to -- `Shelf::Author` for the feed of authors
fn shelves(
    base: &str,
    lib: &str,
    title: &str,
    shelf: fn(i32) -> Shelf,
    categories: &[calibre::Category],
    modified: String,
) -> Feed {
    let root = root_of(base, lib);
    let navigation = categories
        .iter()
        .map(|category| {
            Link::new(page_url(&root, &shelf(category.id).path(), 1))
                .rel("subsection")
                .mime(FEED)
                .title(category.name.clone())
                .count(category.books)
        })
        .collect();

    library_feed(
        format!("{} | {}", lib, title),
        page_url(&root, feed_of(shelf), 1),
        base,
        lib,
    )
    .modified(modified)
    .navigation(navigation)
}

/// Who the library has books by.
#[actix_web::get("/v2/{lib}/authors")]
async fn authors(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match library(&data, &lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let entries = match calibre::authors_with_books(&db) {
        Ok(entries) => entries,
        Err(e) => return server_error("Error querying authors", e),
    };

    let base = origin(&req, data.config);
    json(
        &shelves(&base, &lib, "Authors", Shelf::Author, &entries, calibre::updated(&db)),
        FEED,
    )
}

#[actix_web::get("/v2/{lib}/tags")]
async fn tags(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match library(&data, &lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let entries = match calibre::tags_with_books(&db) {
        Ok(entries) => entries,
        Err(e) => return server_error("Error querying tags", e),
    };

    let base = origin(&req, data.config);
    json(
        &shelves(&base, &lib, "Tags", Shelf::Tag, &entries, calibre::updated(&db)),
        FEED,
    )
}

/// a plain `series` would shadow bindings of that name in this module (same as `books` vs `all_books`)
#[actix_web::get("/v2/{lib}/series")]
async fn all_series(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match library(&data, &lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let entries = match calibre::series_with_books(&db) {
        Ok(entries) => entries,
        Err(e) => return server_error("Error querying series", e),
    };

    let base = origin(&req, data.config);
    json(
        &shelves(&base, &lib, "Series", Shelf::Series, &entries, calibre::updated(&db)),
        FEED,
    )
}

#[actix_web::get("/v2/{lib}/publishers")]
async fn publishers(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match library(&data, &lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let entries = match calibre::publishers_with_books(&db) {
        Ok(entries) => entries,
        Err(e) => return server_error("Error querying publishers", e),
    };

    let base = origin(&req, data.config);
    json(
        &shelves(&base, &lib, "Publishers", Shelf::Publisher, &entries, calibre::updated(&db)),
        FEED,
    )
}

/// A shelf from a custom landing page. Books are paged.
/// a `new` section is the newest of them, capped and unpaged (same as `/new`)
#[actix_web::get("/v2/{lib}/shelves/{slug}")]
async fn named_shelf(
    data: web::Data<AppState>,
    path: web::Path<(String, String)>,
    query: web::Query<PageQuery>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, slug) = path.into_inner();

    if let Err(response) = library(&data, &lib) {
        return response;
    }

    let Some(section) = data.landing(&lib).shelf(&slug) else {
        return HttpResponse::NotFound().body(format!("No shelf named '{}' in {}", slug, lib));
    };

    match (section.kind, section.shelf()) {
        (Kind::New, _) => newest_feed(&data, &req, &lib, section),
        (_, Some(shelf)) => books_feed(&data, &req, &lib, shelf, query.page.unwrap_or(1)),
        (_, None) => HttpResponse::NotFound().body(format!("No shelf named '{}' in {}", slug, lib)),
    }
}

/// The newest books of one shelf: filtered first, capped second.
fn newest_feed(data: &AppState, req: &HttpRequest, lib: &str, section: &Section) -> HttpResponse {
    let db = match library(data, lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let (named, all) = section.tags();
    let books = match calibre::newest_tagged(&db, named, all) {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    let base = origin(req, data.config);
    let newest = library_feed(
        format!("{} | {}", lib, section.title),
        page_url(&root_of(&base, lib), &section.feed(), 1),
        &base,
        lib,
    )
    .modified(calibre::updated(&db));

    json(&holding(newest, &books, lib, &base), FEED)
}

/// The newest arrivals, in the order they arrived.
#[actix_web::get("/v2/{lib}/new")]
async fn recently_added(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match library(&data, &lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let books = match calibre::recently_added(&db) {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    let base = origin(&req, data.config);
    let new = library_feed(
        format!("{} | Recently Added", lib),
        format!("{}/v2/{}/new", base, lib),
        &base,
        &lib,
    )
    .modified(calibre::updated(&db));

    json(&holding(new, &books, &lib, &base), FEED)
}

/// A single book, outside of any feed. This is what the `self` link of every
/// publication points at, and what a client bookmarks or shares.
#[actix_web::get("/v2/{lib}/book/{id}")]
async fn single_book(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, id) = path.into_inner();
    let db = match library(&data, &lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let book = match calibre::book(&db, id) {
        Ok(book) => book,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return HttpResponse::NotFound().body("Book not found")
        }
        Err(e) => return server_error("Error querying book", e),
    };

    let base = origin(&req, data.config);
    json(&publication(&book, &lib, &base), PUBLICATION)
}
