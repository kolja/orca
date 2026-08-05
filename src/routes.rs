
use actix_web::{web, Error, HttpRequest, HttpResponse, Responder};
use actix_web::http::header::{ContentDisposition, DispositionType};
use actix_files as fs;
use tera::Tera;
use crate::authorized::Authorized;
use crate::appstate::AppState;
use crate::calibre;
use crate::config::Config;

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

fn feed_id(path: &str) -> String {
    match path.trim_matches('/') {
        "" => "urn:orca:root".to_string(),
        path => format!("urn:orca:{}", path.replace('/', ":")),
    }
}

/// config plus the absolute URL of this feed for `<link rel="self">`.
/// Some clients resolve the relative hrefs of entries against the self link.
fn feed_ctx(req: &HttpRequest, config: &Config, lib: Option<&str>) -> tera::Context {
    let mut ctx = tera::Context::new();
    let origin = origin(req, config);
    ctx.insert("self_url", &format!("{}{}", origin, req.path()));
    ctx.insert("base", &origin);
    ctx.insert("feed_id", &feed_id(req.path()));
    ctx.insert("author", config.author(lib));
    ctx.insert("version", env!("CARGO_PKG_VERSION"));
    ctx.insert("repository", env!("CARGO_PKG_REPOSITORY"));
    ctx.insert("config", config);
    if let Some(lib) = lib {
        ctx.insert("lib", lib);
    }
    ctx
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

fn render_template(template: &Tera, name: &str, ctx: tera::Context) -> HttpResponse {
    match template.render(name, &ctx) {
        Ok(body) => HttpResponse::Ok()
            .content_type("application/atom+xml")
            .body(body),
        Err(e) => {
            eprintln!("Template rendering error: {}", e);
            HttpResponse::InternalServerError()
                .content_type("application/atom+xml")
                .body("Template rendering error")
        }
    }
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

fn attachment(path: &str) -> Result<fs::NamedFile, Error> {
    let file = fs::NamedFile::open(path).map_err(actix_web::error::ErrorInternalServerError)?;
    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        }))
}

#[actix_web::get("/health")]
async fn health(_auth: Authorized) -> impl Responder {
    HttpResponse::Ok().body("OK")
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

#[actix_web::get("/")]
async fn index(data: web::Data<AppState>, _auth: Authorized, req: HttpRequest) -> impl Responder {
    let libraries: Vec<String> = data.db.keys().cloned().collect();

    if libraries.len() == 1 {
        let lib = &libraries[0];
        return HttpResponse::Found()
            .append_header(("Location", format!("/{}", lib)))
            .finish();
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

#[actix_web::get("/{lib}")]
async fn opds(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => calibre::lock(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let mut ctx = feed_ctx(&req, data.config, Some(&lib));
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "opds.xml.tera", ctx)
}

#[actix_web::get("/{lib}/tags")]
async fn tags(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => calibre::lock(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let tags = match calibre::tags(&db) {
        Ok(tags) => tags,
        Err(e) => return server_error("Error querying tags", e),
    };

    let mut ctx = feed_ctx(&req, data.config, Some(&lib));
    ctx.insert("tags", &tags);
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "tags.xml.tera", ctx)
}

#[actix_web::get("{lib}/tags/{id}")]
async fn books_by_tag(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, tag) = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => calibre::lock(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let books = match calibre::books_by_tag(&db, tag) {
        Ok(books) => wrapped(books),
        Err(e) => return server_error("Error querying books", e),
    };

    let mut ctx = feed_ctx(&req, data.config, Some(&lib));
    ctx.insert("feed_title", &format!("{} | {} books", lib, books.len()));
    ctx.insert("books", &books);
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::get("{lib}/authors")]
async fn authors(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => calibre::lock(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let authors = match calibre::authors(&db) {
        Ok(authors) => authors,
        Err(e) => return server_error("Error querying authors", e),
    };

    let mut ctx = feed_ctx(&req, data.config, Some(&lib));
    ctx.insert("authors", &authors);
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "authors.xml.tera", ctx)
}

#[actix_web::get("{lib}/authors/{id}")]
async fn books_by_author(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, author) = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => calibre::lock(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let books = match calibre::books_by_author(&db, author) {
        Ok(books) => wrapped(books),
        Err(e) => return server_error("Error querying books", e),
    };

    let mut ctx = feed_ctx(&req, data.config, Some(&lib));
    ctx.insert("feed_title", &format!("{} | {} books", lib, books.len()));
    ctx.insert("books", &books);
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::get("{lib}/books")]
async fn getbooks(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => calibre::lock(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let books = match calibre::books(&db) {
        Ok(books) => wrapped(books),
        Err(e) => return server_error("Error querying books", e),
    };

    let mut ctx = feed_ctx(&req, data.config, Some(&lib));
    ctx.insert("feed_title", &format!("{} | {} books", lib, books.len()));
    ctx.insert("books", &books);
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::get("{lib}/new")]
async fn recently_added(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let lib = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => calibre::lock(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let books = match calibre::recently_added(&db) {
        Ok(books) => wrapped(books),
        Err(e) => return server_error("Error querying books", e),
    };

    let mut ctx = feed_ctx(&req, data.config, Some(&lib));
    ctx.insert("feed_title", &format!("{} | Recently Added", lib));
    ctx.insert("books", &books);
    ctx.insert("updated", &calibre::updated(&db));
    render_template(&data.templates, "books.xml.tera", ctx)
}
