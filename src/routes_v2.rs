
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
    BelongsTo, BookMetadata, Contributor, Feed, Link, Publication, Series, ACQUISITION, BOOK, FEED,
    PUBLICATION, SORT_NEW,
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

/// The address of one page of the book feed. Page one keeps the bare URL, so
/// that the link a client bookmarks and the `first` link it is given agree.
fn page_url(base: &str, lib: &str, page: usize) -> String {
    match page {
        1 => format!("{}/v2/{}/books", base, lib),
        n => format!("{}/v2/{}/books?page={}", base, lib, n),
    }
}

/// The links from one page of the book feed to its neighbours.
fn page_links(base: &str, lib: &str, window: &Window) -> Vec<Link> {
    let sibling = |rel: &str, number: usize| {
        Link::new(page_url(base, lib, number))
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
            author: book
                .authors
                .iter()
                .map(|author| Contributor {
                    name: author.name.clone(),
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
            subject: book.tags.iter().map(|tag| tag.name.clone()).collect(),
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

    let total = match calibre::count_books(&db) {
        Ok(total) => total,
        Err(e) => return server_error("Error counting books", e),
    };

    let base = origin(&req, data.config);
    let navigation = vec![
        Link::new(page_url(&base, &lib, 1))
            .rel("subsection")
            .mime(FEED)
            .title("All Books")
            .count(total),
        Link::new(format!("{}/v2/{}/new", base, lib))
            .rel(SORT_NEW)
            .mime(FEED)
            .title("Recently Added"),
    ];

    let root = feed(lib.clone(), format!("{}/v2/{}", base, lib), &base)
        .modified(calibre::updated(&db))
        .navigation(navigation);

    json(&root, FEED)
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
    let db = match library(&data, &lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    let total = match calibre::count_books(&db) {
        Ok(total) => total,
        Err(e) => return server_error("Error counting books", e),
    };
    let window = window(total, PER_PAGE, query.page.unwrap_or(1));

    let books = match calibre::books_page(&db, PER_PAGE, window.offset) {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    let base = origin(&req, data.config);
    let mut page = feed(
        format!("{} | All Books", lib),
        page_url(&base, &lib, window.current),
        &base,
    )
    .modified(calibre::updated(&db))
    .page(total, PER_PAGE, window.current)
    .publications(
        books
            .iter()
            .map(|book| publication(book, &lib, &base))
            .collect(),
    );

    for link in page_links(&base, &lib, &window) {
        page = page.link(link);
    }

    json(&page, FEED)
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
            page_links("https://books.example", "lib", &window(total, 50, requested))
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
        let links = page_links("https://books.example", "lib", &window(120, 50, 2));
        let href = |n: usize| links[n].href.as_str();

        assert_eq!(href(0), "https://books.example/v2/lib/books");
        assert_eq!(href(1), "https://books.example/v2/lib/books");
        assert_eq!(href(2), "https://books.example/v2/lib/books?page=3");
        assert_eq!(href(3), "https://books.example/v2/lib/books?page=3");
    }

    #[test]
    fn only_the_first_page_wears_a_bare_url() {
        assert_eq!(page_url("https://books.example", "lib", 1), "https://books.example/v2/lib/books");
        assert_eq!(page_url("https://books.example", "lib", 2), "https://books.example/v2/lib/books?page=2");
    }
}
