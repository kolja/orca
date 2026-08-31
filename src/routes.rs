
use actix_web::{web, Error, HttpRequest, HttpResponse, Responder};
use actix_web::http::header::{ContentDisposition, DispositionType};
use actix_files as fs;
use serde_derive::{Deserialize, Serialize};
use tera::Tera;
use crate::authdoc;
use crate::authorized::Authorized;
use crate::appstate::AppState;
use crate::calibre;
use crate::config::Config;
use crate::landing::{Kind, Section};
use crate::opds2::SORT_NEW;
use crate::shelf::{self, PageLink, Shelf, PER_PAGE};

const ACQUISITION: &str = "acquisition"; // a feed of books
const NAVIGATION: &str = "navigation"; // a feed of other feeds

/// Where the Atom feeds live (`/v2` is for JSON)
/// cover and a download are the same for both feeds, so those stay directly under `/{lib}`
const V1: &str = "/v1";

/// this is how OPDS 1.2 clients discover search
const OPENSEARCH: &str = "application/opensearchdescription+xml";

#[derive(Deserialize)]
struct PageQuery {
    page: Option<usize>,
}

#[derive(Deserialize)]
struct SearchQuery {
    query: Option<String>,
    page: Option<usize>,
}

/// the OpenSearch counts are what a client reads to say "1-50 of 340".
#[derive(Serialize)]
struct Paging {
    total: usize,
    per_page: usize,
    /// One-based
    start: usize,
    links: Vec<PageLink>,
}

/// The externally visible origin of this request, as `scheme://host` without a
/// trailing slash. `connection_info` honours X-Forwarded-Proto / X-Forwarded-Host,
/// so this stays correct behind a reverse proxy -- could otherwise be resolved to something like
/// 0.0.0.0
pub(crate) fn origin(req: &HttpRequest, config: &Config) -> String {
    match &config.server.public_url {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => {
            let conn = req.connection_info();
            format!("{}://{}", conn.scheme(), conn.host())
        }
    }
}

/// The `v1` segment is left out
fn feed_id(path: &str) -> String {
    let path = path.trim_matches('/');
    let path = match path {
        "v1" => "",
        path => path.strip_prefix("v1/").unwrap_or(path),
    };

    match path {
        "" => "urn:orca:root".to_string(),
        path => format!("urn:orca:{}", path.replace('/', ":")),
    }
}

/// `/` and the bare `/{lib}/…` paths are aliases of `/v1`
/// `Found` rather than `MovedPermanently`
fn moved_to_v1(req: &HttpRequest, config: &Config, path: &str) -> HttpResponse {
    let target = format!("{}{}{}", origin(req, config), V1, path);
    let target = match req.query_string() {
        "" => target,
        query => format!("{}?{}", target, query),
    };

    HttpResponse::Found().append_header(("Location", target)).finish()
}

/// config plus the absolute URL of this feed for `<link rel="self">`.
/// Everything the layout asks about is inserted here
fn feed_ctx(req: &HttpRequest, config: &Config, lib: Option<&str>) -> tera::Context {
    let mut ctx = tera::Context::new();
    let origin = origin(req, config);
    ctx.insert("self_url", &format!("{}{}", origin, req.path()));
    ctx.insert("catalog", &format!("{}{}", origin, V1));
    ctx.insert("base", &origin);
    ctx.insert("feed_id", &feed_id(req.path()));
    ctx.insert("author", config.author(lib));
    ctx.insert("version", env!("CARGO_PKG_VERSION"));
    ctx.insert("repository", env!("CARGO_PKG_REPOSITORY"));
    ctx.insert("config", config);
    ctx.insert("kind", NAVIGATION);
    ctx.insert("up", &false);
    ctx.insert("paging", &Option::<Paging>::None);
    ctx.insert("lib", lib.unwrap_or_default());
    ctx
}

fn shelf_ctx(req: &HttpRequest, config: &Config, lib: &str, kind: &str) -> tera::Context {
    let mut ctx = feed_ctx(req, config, Some(lib));
    ctx.insert("up", &true);
    ctx.insert("kind", kind);
    ctx
}

/// root of the Atom feed (JSON is under `/v2`)
fn root_of(req: &HttpRequest, config: &Config, lib: &str) -> String {
    format!("{}{}/{}", origin(req, config), V1, lib)
}

/// Calibre stores a blurb as HTML - Rendering / escaping happens here
fn wrapped(mut books: Vec<calibre::Book>) -> Vec<calibre::Book> {
    for book in &mut books {
        book.synopsis = calibre::plain_text(&book.synopsis, calibre::SYNOPSIS_WIDTH);
    }
    books
}

/// Log a failure and turn it into a 500
pub(crate) fn server_error(what: &str, e: impl std::fmt::Display) -> HttpResponse {
    eprintln!("{}: {}", what, e);
    HttpResponse::InternalServerError().body(what.to_string())
}

fn rendered(template: &Tera, name: &str, ctx: tera::Context, mime: &str) -> HttpResponse {
    match template.render(name, &ctx) {
        Ok(body) => HttpResponse::Ok().content_type(mime.to_string()).body(body),
        Err(e) => {
            eprintln!("Template rendering error: {}", e);
            HttpResponse::InternalServerError()
                .content_type(mime.to_string())
                .body("Template rendering error")
        }
    }
}

fn render_template(template: &Tera, name: &str, ctx: tera::Context) -> HttpResponse {
    rendered(template, name, ctx, "application/atom+xml")
}

/// The path of a file inside a library, or a 404 for a library Orca does not serve.
fn library_path<'a>(data: &'a AppState, lib: &str) -> Result<&'a str, Error> {
    data.config
        .calibre
        .libraries
        .get(lib)
        .map(|library| library.path.as_str())
        .ok_or_else(|| actix_web::error::ErrorNotFound("Library not found"))
}

/// A book or a cover the client asks for may simply be gone -- a book deleted
/// from Calibre while a client still holds a cached catalog.
fn not_found_or_500(missing: &'static str) -> impl Fn(rusqlite::Error) -> Error {
    move |e| match e {
        rusqlite::Error::QueryReturnedNoRows => actix_web::error::ErrorNotFound(missing),
        e => actix_web::error::ErrorInternalServerError(e),
    }
}

/// The connection of one library, or a 404 for a library Orca does not serve.
fn library<'a>(
    data: &'a AppState,
    lib: &str,
) -> Result<std::sync::MutexGuard<'a, rusqlite::Connection>, HttpResponse> {
    match data.db.get(lib) {
        Some(db) => Ok(calibre::lock(db)),
        None => Err(HttpResponse::NotFound().body(format!("Database '{}' not found", lib))),
    }
}

/// One page of books
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
    let window = shelf::window(total, PER_PAGE, requested);

    let books = match shelf.books(&db, PER_PAGE, window.offset) {
        Ok(books) => wrapped(books),
        Err(e) => return server_error("Error querying books", e),
    };

    let root = root_of(req, data.config, lib);
    let path = shelf.path();
    let mut ctx = shelf_ctx(req, data.config, lib, ACQUISITION);
    // `self` names the page that answered, not the feed it is one page of.
    ctx.insert("self_url", &shelf::page_url(&root, &path, window.current));
    ctx.insert("feed_title", &format!("{} | {}", lib, name));
    ctx.insert("books", &books);
    ctx.insert("updated", &calibre::updated(&db));
    ctx.insert(
        "paging",
        &Paging {
            total,
            per_page: PER_PAGE,
            start: window.offset + 1,
            links: shelf::page_links(&root, &path, &window),
        },
    );
    render_template(&data.templates, "books.xml.tera", ctx)
}

/// One entry per category, e.g. `Shelf::Author` for the feed of authors.
fn shelves_feed(
    data: &AppState,
    req: &HttpRequest,
    lib: &str,
    title: &str,
    shelf: fn(i32) -> Shelf,
    categories: &[calibre::Category],
    updated: String,
) -> HttpResponse {
    let mut ctx = shelf_ctx(req, data.config, lib, NAVIGATION);
    ctx.insert("feed_title", &format!("{} | {}", lib, title));
    ctx.insert("shelf", shelf::feed_of(shelf));
    ctx.insert("noun", shelf::noun_of(shelf));
    ctx.insert("entries", categories);
    ctx.insert("updated", &updated);
    render_template(&data.templates, "shelves.xml.tera", ctx)
}

fn attachment(path: &str) -> Result<fs::NamedFile, Error> {
    let file = fs::NamedFile::open(path).map_err(actix_web::error::ErrorInternalServerError)?;
    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        }))
}

/// public, like `/auth`
#[actix_web::get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().body("OK")
}

/// for a client that would rather show a login form of its own
/// than send credentials blind. The same document every 401 carries.
#[actix_web::get("/auth")]
async fn authentication(data: web::Data<AppState>, req: HttpRequest) -> impl Responder {
    HttpResponse::Ok()
        .content_type(authdoc::AUTHENTICATION)
        .body(authdoc::body(&origin(&req, data.config)))
}

/// The logo for the login form
#[actix_web::get("/auth/logo.svg")]
async fn logo() -> impl Responder {
    HttpResponse::Ok()
        .content_type(authdoc::LOGO)
        .body(&include_bytes!("../orca-logo.svg")[..])
}

#[actix_web::get("/{lib}/cover/{id}")]
async fn cover(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    _auth: Authorized,
    _req: HttpRequest,
) -> Result<fs::NamedFile, Error> {
    let (lib, book) = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => calibre::lock(db),
        None => return Err(actix_web::error::ErrorNotFound("Library not found")),
    };
    let library = library_path(&data, &lib)?;

    let cover = calibre::cover_path(&db, book).map_err(not_found_or_500("Cover not found"))?;

    attachment(&format!("{}/{}", library, cover))
}

#[actix_web::get("/{lib}/file/{id}/{format}")]
async fn book_file(
    data: web::Data<AppState>,
    path: web::Path<(String, i32, String)>,
    _auth: Authorized,
    _req: HttpRequest,
) -> Result<fs::NamedFile, Error> {
    let (lib, book, format) = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => calibre::lock(db),
        None => return Err(actix_web::error::ErrorNotFound("Library not found")),
    };
    let library = library_path(&data, &lib)?;

    let file =
        calibre::file_path(&db, book, &format).map_err(not_found_or_500("Book not found"))?;

    attachment(&format!("{}/{}", library, file))
}

#[actix_web::get("/v1")]
async fn index(data: web::Data<AppState>, auth: Authorized, req: HttpRequest) -> impl Responder {
    let mut libraries: Vec<String> = data
        .db
        .keys()
        .filter(|lib| auth.may_reach(data.config, &format!("{}/{}", V1, lib)))
        .cloned()
        .collect();
    libraries.sort();

    if libraries.len() == 1 {
        return moved_to_v1(&req, data.config, &format!("/{}", libraries[0]));
    }

    let updated = data
        .db
        .values()
        .map(|db| calibre::updated(&calibre::lock(db)))
        .max()
        .unwrap_or_else(|| "2000-01-01T00:00:00+00:00".to_string());

    let mut ctx = feed_ctx(&req, data.config, None);
    ctx.insert("libraries", &libraries);
    ctx.insert("updated", &updated);
    render_template(&data.templates, "index.xml.tera", ctx)
}

/// One section of the landing page
#[derive(Serialize)]
struct Entry {
    title: String,
    /// the tail of the entry's urn
    id: String,
    href: String,
    /// only Recently Added has one
    rel: &'static str,
    /// `acquisition` for a feed of books, `navigation` for a feed of links
    kind: &'static str,
    content: String,
}

#[actix_web::get("/v1/{lib}")]
async fn opds(
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

    let page = match data.landing(&lib).visible(&db, &counts) {
        Ok(page) => page,
        Err(e) => return server_error("Error reading the landing page", e),
    };

    let root = root_of(&req, data.config, &lib);
    let sections: Vec<Entry> = page
        .iter()
        .map(|section| Entry {
            title: section.title.clone(),
            // `shelves/scifi` is one feed, and an urn spells a path with colons.
            id: section.feed().replace('/', ":"),
            href: format!("{}/{}", root, section.feed()),
            rel: match section.kind {
                Kind::New => SORT_NEW,
                _ => "",
            },
            kind: match section.kind.is_shelf() {
                true => ACQUISITION,
                false => NAVIGATION,
            },
            content: section.description(&counts),
        })
        .collect();

    let mut ctx = feed_ctx(&req, data.config, Some(&lib));
    ctx.insert("sections", &sections);
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "opds.xml.tera", ctx)
}

/// The OpenSearch description document. An OPDS 1.2 client discovers search this way.
#[actix_web::get("/v1/{lib}/opensearch.xml")]
async fn opensearch(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    if let Err(response) = library(&data, &lib) {
        return response;
    }

    let ctx = feed_ctx(&req, data.config, Some(&lib));
    rendered(&data.templates, "opensearch.xml.tera", ctx, OPENSEARCH)
}

#[actix_web::get("/v1/{lib}/tags")]
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

    shelves_feed(&data, &req, &lib, "Tags", Shelf::Tag, &entries, calibre::updated(&db))
}

#[actix_web::get("/v1/{lib}/authors")]
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

    shelves_feed(&data, &req, &lib, "Authors", Shelf::Author, &entries, calibre::updated(&db))
}

/// `all_series` rather than `series`:
/// a plural that is also a singular would shadow every binding of that name.
#[actix_web::get("/v1/{lib}/series")]
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

    shelves_feed(&data, &req, &lib, "Series", Shelf::Series, &entries, calibre::updated(&db))
}

#[actix_web::get("/v1/{lib}/publishers")]
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

    shelves_feed(&data, &req, &lib, "Publishers", Shelf::Publisher, &entries, calibre::updated(&db))
}

#[actix_web::get("/v1/{lib}/books")]
async fn getbooks(
    data: web::Data<AppState>,
    path: web::Path<String>,
    query: web::Query<PageQuery>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    books_feed(&data, &req, &lib, Shelf::Everything, query.page.unwrap_or(1))
}

#[actix_web::get("/v1/{lib}/tags/{id}")]
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

#[actix_web::get("/v1/{lib}/authors/{id}")]
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

/// One series, in reading order.
#[actix_web::get("/v1/{lib}/series/{id}")]
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

#[actix_web::get("/v1/{lib}/publishers/{id}")]
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

/// What the OpenSearch description points at. `query` rather than `q`
#[actix_web::get("/v1/{lib}/search")]
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

/// A shelf from a custom landing page. Books are paged.
#[actix_web::get("/v1/{lib}/shelves/{slug}")]
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

/// The newest books of one shelf: first filtered, then capped.
fn newest_feed(data: &AppState, req: &HttpRequest, lib: &str, section: &Section) -> HttpResponse {
    let db = match library(data, lib) {
        Ok(db) => db,
        Err(response) => return response,
    };

    // Not `tags`: the handler of that name is a unit struct, and a pattern would match it rather than bind.
    let (named, all) = section.tags();
    let books = match calibre::newest_tagged(&db, named, all) {
        Ok(books) => wrapped(books),
        Err(e) => return server_error("Error querying books", e),
    };

    let mut ctx = shelf_ctx(req, data.config, lib, ACQUISITION);
    ctx.insert("feed_title", &format!("{} | {}", lib, section.title));
    ctx.insert("books", &books);
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "books.xml.tera", ctx)
}

/// The newest arrivals, in the order they arrived. A window on the library
/// rather than a shelf of it, so it is not paged and carries no page links.
#[actix_web::get("/v1/{lib}/new")]
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
        Ok(books) => wrapped(books),
        Err(e) => return server_error("Error querying books", e),
    };

    let mut ctx = shelf_ctx(&req, data.config, &lib, ACQUISITION);
    ctx.insert("feed_title", &format!("{} | Recently Added", lib));
    ctx.insert("books", &books);
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::get("/")]
async fn catalog_moved(
    data: web::Data<AppState>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    moved_to_v1(&req, data.config, "")
}

/// Only a path that could once have been an OPDS 1.2 one is an alias of `/v1`.
fn alias(req: &HttpRequest, config: &Config, lib: &str, path: &str) -> HttpResponse {
    match crate::RESERVED.contains(&lib) {
        true => HttpResponse::NotFound().body(format!("No such feed: {}", path)),
        false => moved_to_v1(req, config, path),
    }
}

/// Where OPDS 1.2 used to live. Registered after the covers and the downloads,
#[actix_web::get("/{lib}")]
async fn library_moved(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    alias(&req, data.config, &lib, &format!("/{}", lib))
}

#[actix_web::get("/{lib}/{feed:.*}")]
async fn feed_moved(
    data: web::Data<AppState>,
    path: web::Path<(String, String)>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, feed) = path.into_inner();
    alias(&req, data.config, &lib, &format!("/{}/{}", lib, feed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_feed_is_identified_by_what_it_holds_not_by_where_it_is() {
        assert_eq!(feed_id("/v1"), "urn:orca:root");
        assert_eq!(feed_id("/v1/library"), "urn:orca:library");
        assert_eq!(feed_id("/v1/library/tags/5"), "urn:orca:library:tags:5");

        // The ids these produced before the feeds moved into `/v1`, unchanged.
        assert_eq!(feed_id("/"), "urn:orca:root");
        assert_eq!(feed_id("/library/tags/5"), "urn:orca:library:tags:5");
    }

    // Only the segment, never the prefix: `v1` is reserved, `v1x` is a library
    // anyone may have.
    #[test]
    fn a_library_whose_name_begins_with_v1_keeps_it() {
        assert_eq!(feed_id("/v1/v1x/books"), "urn:orca:v1x:books");
        assert_eq!(feed_id("/v1x/books"), "urn:orca:v1x:books");
    }
}
