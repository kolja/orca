
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
use crate::calibre::{self, Book};
use crate::opds2::{
    BelongsTo, BookMetadata, Contributor, Feed, Link, Publication, Series, Subject, ACQUISITION,
    BOOK, FEED, PUBLICATION, SORT_NEW,
};
use crate::routes::{origin, server_error};

/// How many books one page of the catalog holds.
const PER_PAGE: usize = 50;

#[derive(Deserialize)]
struct PageQuery {
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

fn json(value: &impl Serialize, mime: &str) -> HttpResponse {
    match serde_json::to_string(value) {
        Ok(body) => HttpResponse::Ok().content_type(mime).body(body),
        Err(e) => server_error("Error serialising feed", e),
    }
}

/// Which slice of the library a request for `page` asks for.
/// Pages count from one, and one past the end is the last page: An empty library still has a page one.
fn window(total: usize, per_page: usize, requested: usize) -> Window {
    let last = total.div_ceil(per_page).max(1);
    let current = requested.clamp(1, last);
    Window {
        current,
        last,
        offset: (current - 1) * per_page,
    }
}

struct Window {
    current: usize,
    last: usize,
    offset: usize,
}

/// The address of one page of a feed. Page one keeps the bare URL
fn page_url(base: &str, lib: &str, feed: &str, page: usize) -> String {
    match page {
        1 => format!("{}/v2/{}/{}", base, lib, feed),
        n => format!("{}/v2/{}/{}?page={}", base, lib, feed, n),
    }
}

/// The links from one page of a book feed to its neighbours.
fn page_links(base: &str, lib: &str, feed: &str, window: &Window) -> Vec<Link> {
    let sibling = |rel: &str, number: usize| {
        Link::new(page_url(base, lib, feed, number))
            .rel(rel.to_string())
            .mime(FEED)
    };

    let mut links = Vec::new();
    if window.current > 1 {
        links.push(sibling("first", 1));
        links.push(sibling("previous", window.current - 1));
    }
    if window.current < window.last {
        links.push(sibling("next", window.current + 1));
        links.push(sibling("last", window.last));
    }
    links
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
        Link::new(format!("{}/{}/file/{}/{}", base, lib, book.id, format))
            .rel(ACQUISITION)
            .mime(calibre::mime(format))
            .title(format!("{}.{}", book.title, format))
    }));

    let images = match book.has_cover {
        true => vec![Link::new(format!("{}/{}/cover/{}", base, lib, book.id)).mime("image/jpeg")],
        false => Vec::new(),
    };

    Publication {
        metadata: BookMetadata {
            kind: BOOK,
            title: book.title.clone(),
            identifier: Some(identifier(book, lib)),
            // link from a book to the rest of what its author wrote.
            author: book
                .authors
                .iter()
                .map(|author| Contributor {
                    name: author.name.clone(),
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
            publisher: book.publisher.clone(),
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
                    position: Some(series.index),
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
async fn catalog(data: web::Data<AppState>, _auth: Authorized, req: HttpRequest) -> impl Responder {
    let mut libraries: Vec<&String> = data.db.keys().collect();
    libraries.sort();

    if let [only] = libraries[..] {
        return HttpResponse::Found()
            .append_header(("Location", format!("/v2/{}", only)))
            .finish();
    }

    let base = origin(&req, data.config);
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
    let mut root = feed("ORCA", format!("{}/v2", base), &base).navigation(navigation);
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
    let browse = |feed: &str, title: &str, count: usize| {
        Link::new(page_url(&base, &lib, feed, 1))
            .rel("subsection")
            .mime(FEED)
            .title(title.to_string())
            .count(count)
    };

    let mut navigation = vec![
        browse(Shelf::Everything.feed(), "All Books", counts.books),
        Link::new(format!("{}/v2/{}/new", base, lib))
            .rel(SORT_NEW)
            .mime(FEED)
            .title("Recently Added"),
    ];

    // A library nobody has tagged should not offer a way in that leads nowhere.
    if counts.authors > 0 {
        navigation.push(browse(feed_of(Shelf::Author), "Authors", counts.authors));
    }
    if counts.tags > 0 {
        navigation.push(browse(feed_of(Shelf::Tag), "Tags", counts.tags));
    }

    let root = feed(lib.clone(), format!("{}/v2/{}", base, lib), &base)
        .modified(calibre::updated(&db))
        .navigation(navigation);

    json(&root, FEED)
}

/// a `shelf` carries no books -- it says only how to ask Calibre and what to call the result.
/// paging works identical on all of them.
enum Shelf {
    Everything,
    Author(i32),
    Tag(i32),
}

impl Shelf {
    /// The feed every shelf of this kind lives in, below the library. The only
    /// place these path segments are spelled out.
    fn feed(&self) -> &'static str {
        match self {
            Shelf::Everything => "books",
            Shelf::Author(_) => "authors",
            Shelf::Tag(_) => "tags",
        }
    }

    /// Where this one shelf lives below the library.
    fn path(&self) -> String {
        match self {
            Shelf::Everything => self.feed().to_string(),
            Shelf::Author(id) | Shelf::Tag(id) => format!("{}/{}", self.feed(), id),
        }
    }

    /// What to call this feed. `QueryReturnedNoRows` for a shelf the library does not have.
    fn name(&self, db: &Connection) -> rusqlite::Result<String> {
        match self {
            Shelf::Everything => Ok("All Books".to_string()),
            Shelf::Author(id) => calibre::author_name(db, *id),
            Shelf::Tag(id) => calibre::tag_name(db, *id),
        }
    }

    fn count(&self, db: &Connection) -> rusqlite::Result<usize> {
        match self {
            Shelf::Everything => calibre::count_books(db),
            Shelf::Author(id) => calibre::count_books_by_author(db, *id),
            Shelf::Tag(id) => calibre::count_books_by_tag(db, *id),
        }
    }

    fn books(&self, db: &Connection, limit: usize, offset: usize) -> rusqlite::Result<Vec<Book>> {
        match self {
            Shelf::Everything => calibre::books_page(db, limit, offset),
            Shelf::Author(id) => calibre::books_by_author_page(db, *id, limit, offset),
            Shelf::Tag(id) => calibre::books_by_tag_page(db, *id, limit, offset),
        }
    }
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
    let path = shelf.path();
    let mut page = feed(
        format!("{} | {}", lib, name),
        page_url(&base, lib, &path, window.current),
        &base,
    )
    .modified(calibre::updated(&db))
    .page(total, PER_PAGE, window.current)
    .publications(books.iter().map(|book| publication(book, lib, &base)).collect());

    for link in page_links(&base, lib, &path, &window) {
        page = page.link(link);
    }

    json(&page, FEED)
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

/// The feed a kind of shelf lives in: `authors` for `Shelf::Author`
fn feed_of(shelf: fn(i32) -> Shelf) -> &'static str {
    shelf(0).feed()
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
    let navigation = categories
        .iter()
        .map(|category| {
            Link::new(page_url(base, lib, &shelf(category.id).path(), 1))
                .rel("subsection")
                .mime(FEED)
                .title(category.name.clone())
                .count(category.books)
        })
        .collect();

    feed(
        format!("{} | {}", lib, title),
        page_url(base, lib, feed_of(shelf), 1),
        base,
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
    let new = feed(
        format!("{} | Recently Added", lib),
        format!("{}/v2/{}/new", base, lib),
        &base,
    )
    .modified(calibre::updated(&db))
    .publications(
        books
            .iter()
            .map(|book| publication(book, &lib, &base))
            .collect(),
    );

    json(&new, FEED)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_that_fits_on_one_page_has_only_that_page() {
        let one = window(4, 50, 1);
        assert_eq!((one.current, one.last, one.offset), (1, 1, 0));

        // A library with nothing in it is still a page, not a 404.
        let empty = window(0, 50, 1);
        assert_eq!((empty.current, empty.last, empty.offset), (1, 1, 0));
    }

    #[test]
    fn pages_cover_the_library_exactly() {
        // 120 books, 50 to a page: 50 + 50 + 20.
        assert_eq!(window(120, 50, 1).offset, 0);
        assert_eq!(window(120, 50, 2).offset, 50);
        assert_eq!(window(120, 50, 3).offset, 100);
        assert_eq!(window(120, 50, 3).last, 3);

        // A page that divides evenly does not add an empty one at the end.
        assert_eq!(window(100, 50, 1).last, 2);
    }

    // A `next` link a client kept from before half the library was deleted.
    #[test]
    fn a_page_past_the_end_is_the_last_page() {
        let past = window(120, 50, 99);
        assert_eq!((past.current, past.last, past.offset), (3, 3, 100));
    }

    // `?page=0` should not underflow.
    #[test]
    fn a_page_before_the_beginning_is_the_first_page() {
        assert_eq!(window(120, 50, 0).current, 1);
        assert_eq!(window(120, 50, 0).offset, 0);
    }

    // The fixture library fits on one page, so this is where the way from one
    // page to the next is checked.
    #[test]
    fn a_page_links_to_the_pages_around_it() {
        let rels = |total, requested| {
            page_links("https://books.example", "lib", "books", &window(total, 50, requested))
                .iter()
                .map(|link| link.rel.clone().unwrap_or_default())
                .collect::<Vec<_>>()
        };

        assert_eq!(rels(120, 2), ["first", "previous", "next", "last"]);
        assert_eq!(rels(120, 1), ["next", "last"]);
        assert_eq!(rels(120, 3), ["first", "previous"]);
        // Nowhere else to go.
        assert!(rels(4, 1).is_empty());
    }

    #[test]
    fn the_pages_around_a_page_are_the_ones_beside_it() {
        let links = page_links("https://books.example", "lib", "books", &window(120, 50, 2));
        let href = |n: usize| links[n].href.as_str();

        assert_eq!(href(0), "https://books.example/v2/lib/books");
        assert_eq!(href(1), "https://books.example/v2/lib/books");
        assert_eq!(href(2), "https://books.example/v2/lib/books?page=3");
        assert_eq!(href(3), "https://books.example/v2/lib/books?page=3");
    }

    #[test]
    fn only_the_first_page_wears_a_bare_url() {
        assert_eq!(page_url("https://books.example", "lib", "books", 1), "https://books.example/v2/lib/books");
        assert_eq!(page_url("https://books.example", "lib", "books", 2), "https://books.example/v2/lib/books?page=2");
    }

    // A shelf is paged the same way the library is, and its pages are pages of
    // the shelf rather than of the catalog.
    #[test]
    fn a_shelf_pages_under_its_own_address() {
        let path = Shelf::Author(5).path();
        assert_eq!(path, "authors/5");
        assert_eq!(Shelf::Tag(9).path(), "tags/9");
        assert_eq!(Shelf::Everything.path(), "books");

        assert_eq!(
            page_url("https://books.example", "lib", &path, 2),
            "https://books.example/v2/lib/authors/5?page=2"
        );
    }
}
