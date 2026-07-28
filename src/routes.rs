
use actix_web::{web, Error, HttpRequest, HttpResponse, Responder};
use actix_web::http::header::{ContentDisposition, DispositionType};
use actix_files as fs;
use tera::Tera;
use serde_derive::Serialize;
use html2text::from_read;
use rusqlite::{params, Connection, Row};
use std::sync::{Mutex, MutexGuard};
use crate::authorized::Authorized;
use crate::appstate::AppState;
use crate::config::Config;

#[derive(Debug, Serialize)]
struct Book {
    id: i32,
    title: String,
    pubdate: String,
    synopsis: String,
    author_id: i32,
    author_name: String,
    book_file: Option<String>,
    formats: Vec<Format>,
}

#[derive(Debug, Serialize)]
struct Author {
    id: i32,
    name: String,
}

#[derive(Debug, Serialize)]
struct Tag {
    id: i32,
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    EPUB,
    PDF,
    MOBI,
}

impl Format {
    fn from_str(s: &str) -> Option<Format> {
        match s.to_lowercase().as_str() {
            "epub" => Some(Format::EPUB),
            "pdf" => Some(Format::PDF),
            "mobi" => Some(Format::MOBI),
            _ => None,
        }
    }
}
/// The externally visible origin of this request, as `scheme://host` without a
/// trailing slash. `connection_info` honours X-Forwarded-Proto / X-Forwarded-Host,
/// so this stays correct behind a reverse proxy -- could otherwise be resolved to something like
/// 0.0.0.0
fn origin(req: &HttpRequest, config: &Config) -> String {
    match &config.server.public_url {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => {
            let conn = req.connection_info();
            format!("{}://{}", conn.scheme(), conn.host())
        }
    }
}

/// Context common to every feed: the config plus the absolute URL of this feed
/// for `<link rel="self">`. Some clients resolve the relative
/// hrefs of entries against the self link rather than against the URL they
/// fetched, so the self link has to be both absolute and per-request.
fn feed_ctx(req: &HttpRequest, config: &Config) -> tera::Context {
    let mut ctx = tera::Context::new();
    let origin = origin(req, config);
    ctx.insert("self_url", &format!("{}{}", origin, req.path()));
    ctx.insert("base", &origin);
    ctx.insert("config", config);
    ctx
}

/// Take the connection lock, recovering from poisoning
fn lock_db(db: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    db.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Log a failure and turn it into a 500
fn server_error(what: &str, e: impl std::fmt::Display) -> HttpResponse {
    eprintln!("{}: {}", what, e);
    HttpResponse::InternalServerError().body(what.to_string())
}

/// Collect the rows that could be read, logging the ones that could not.
fn collect_rows<T>(rows: impl Iterator<Item = rusqlite::Result<T>>, what: &str) -> Vec<T> {
    rows.filter_map(|row| match row {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("Skipping unreadable {} row: {}", what, e);
            None
        }
    })
    .collect()
}

/// Run one of the book queries and map its rows to `Book`s.
fn query_books(
    db: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<Book>> {
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        let synopsis: String = row.get(3).unwrap_or_default();
        let synopsis = from_read(synopsis.as_bytes(), 100).unwrap_or_default();
        let format_str: String = row.get("formats").unwrap_or_default();
        let formats = format_str.split(',').filter_map(Format::from_str).collect();
        Ok(Book {
            id: row.get(0)?,
            title: row.get(1)?,
            pubdate: row.get(2)?,
            synopsis,
            author_name: row.get(4)?,
            author_id: row.get(5)?,
            book_file: row.get(6)?,
            formats,
        })
    })?;
    Ok(collect_rows(rows, "book"))
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
    let (lib, image_id) = path.into_inner();
    let db_lock = match data.db.get(&lib) {
        Some(db) => lock_db(db),
        None => return Err(actix_web::error::ErrorNotFound("Library not found")),
    };
    let library_path = data
        .config
        .calibre
        .libraries
        .get(&lib)
        .ok_or_else(|| actix_web::error::ErrorNotFound("Library not found"))?;

    let mut stmt = db_lock
        .prepare("SELECT books.path FROM books WHERE books.id = ?1 AND books.has_cover = true;")
        .map_err(actix_web::error::ErrorInternalServerError)?;

    // A cover the client asks for may simply be gone — a book deleted from
    // Calibre while a client still holds a cached catalog -> 404
    let path: String = stmt
        .query_row(rusqlite::params![image_id], |row| row.get(0))
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                actix_web::error::ErrorNotFound("Cover not found")
            }
            e => actix_web::error::ErrorInternalServerError(e),
        })?;

    let cover_path = format!("{}/{}/cover.jpg", library_path, path);

    let file = fs::NamedFile::open(&cover_path)
        .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        }))
}

#[actix_web::get("/{lib}/file/{id}/{format}")]
async fn book_file(
    data: web::Data<AppState>,
    path: web::Path<(String, i32, String)>,
    _auth: Authorized,
    _req: HttpRequest,
) -> Result<fs::NamedFile, Error> {
    let (db, id, format) = path.into_inner();
    let db_lock = match data.db.get(&db) {
        Some(db) => lock_db(db),
        None => return Err(actix_web::error::ErrorNotFound("Library not found")),
    };
    let library_path = data
        .config
        .calibre
        .libraries
        .get(&db)
        .ok_or_else(|| actix_web::error::ErrorNotFound("Library not found"))?;

    let mut stmt = db_lock
        .prepare(
            "SELECT b.path, d.name AS file
                  FROM books b
                  LEFT JOIN data d ON b.id = d.book
                  WHERE b.id = ?1 GROUP BY b.id;",
        )
        .map_err(actix_web::error::ErrorInternalServerError)?;

    let row_mapper = |row: &Row| -> rusqlite::Result<(String, String)> {
        let path: String = row.get(0)?;
        let file: String = row.get(1)?;
        Ok((path, file))
    };

    // Unknown book id, or a book with no file of any format -> 404
    let (path, file): (String, String) = stmt
        .query_row(rusqlite::params![id], row_mapper)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                actix_web::error::ErrorNotFound("Book not found")
            }
            e => actix_web::error::ErrorInternalServerError(e),
        })?;

    let book_file_path = format!("{}/{}/{}.{}", library_path, path, file, format);

    let file = fs::NamedFile::open(&book_file_path)
        .map_err(actix_web::error::ErrorInternalServerError)?;

    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        }))
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

    let mut ctx = feed_ctx(&req, data.config);
    ctx.insert("libraries", &libraries);
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

    let mut ctx = feed_ctx(&req, data.config);
    ctx.insert("lib", &lib);
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
        Some(db) => lock_db(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let mut ctx = feed_ctx(&req, data.config);
    ctx.insert("lib", &lib);

    let mut stmt = match db.prepare("SELECT id, name FROM tags;") {
        Ok(stmt) => stmt,
        Err(e) => return server_error("Error preparing statement", e),
    };

    let tags_iter = match stmt.query_map(params![], |row| {
        Ok(Tag {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    }) {
        Ok(iter) => iter,
        Err(e) => return server_error("Error querying tags", e),
    };

    let tags: Vec<Tag> = collect_rows(tags_iter, "tag");
    ctx.insert("tags", &tags);
    render_template(&data.templates, "tags.xml.tera", ctx)
}

#[actix_web::get("{lib}/tags/{id}")]
async fn books_by_tag(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, tag_id) = path.into_inner();
    let mut ctx = feed_ctx(&req, data.config);
    ctx.insert("lib", &lib);

    let db = match data.db.get(&lib) {
        Some(db) => lock_db(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let books_by_tag = query_books(
        &db,
        "SELECT b.id, b.title, b.pubdate, c.text AS synopsis, a.name AS author_name, a.id AS author_id, d.name AS book_file,
            GROUP_CONCAT(d.format) AS formats
            FROM books b
            JOIN books_tags_link bt ON b.id = bt.book
            JOIN tags t ON bt.tag = t.id
            JOIN books_authors_link ba ON b.id = ba.book
            JOIN authors a ON ba.author = a.id
            LEFT JOIN comments c ON b.id = c.book
            LEFT JOIN data d ON b.id = d.book
            WHERE t.id = ?1 GROUP BY b.id;",
        params![tag_id],
    );
    let books_by_tag = match books_by_tag {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    ctx.insert("books", &books_by_tag);
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

    let mut ctx = feed_ctx(&req, data.config);
    ctx.insert("lib", &lib);

    let db = match data.db.get(&lib) {
        Some(db) => lock_db(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let mut stmt = match db.prepare("SELECT id, name FROM authors;") {
        Ok(stmt) => stmt,
        Err(e) => return server_error("Error preparing statement", e),
    };

    let author_iter = match stmt.query_map(params![], |row| {
        Ok(Author {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    }) {
        Ok(iter) => iter,
        Err(e) => return server_error("Error querying authors", e),
    };

    let authors: Vec<Author> = collect_rows(author_iter, "author");
    ctx.insert("authors", &authors);

    render_template(&data.templates, "authors.xml.tera", ctx)
}

#[actix_web::get("{lib}/authors/{id}")]
async fn books_by_author(
    data: web::Data<AppState>,
    author_id: web::Path<(String, i32)>,
    _auth: Authorized,
    req: HttpRequest,
) -> impl Responder {
    let (lib, author_id) = author_id.into_inner();
    let mut ctx = feed_ctx(&req, data.config);
    ctx.insert("lib", &lib);

    let db = match data.db.get(&lib) {
        Some(db) => lock_db(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let books_by_author = query_books(
        &db,
        "SELECT b.id, b.title, b.pubdate, c.text AS synopsis, a.name AS author_name, a.id AS author_id, d.name AS book_file,
            GROUP_CONCAT(d.format) AS formats
            FROM books b
            JOIN books_tags_link bt ON b.id = bt.book
            JOIN tags t ON bt.tag = t.id
            JOIN books_authors_link ba ON b.id = ba.book
            JOIN authors a ON ba.author = a.id
            LEFT JOIN comments c ON b.id = c.book
            LEFT JOIN data d ON b.id = d.book
            WHERE a.id = ?1 GROUP BY b.id;",
        params![author_id],
    );
    let books_by_author = match books_by_author {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    ctx.insert("books", &books_by_author);
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

    let mut ctx = feed_ctx(&req, data.config);
    ctx.insert("lib", &lib);

    let db = match data.db.get(&lib) {
        Some(db) => lock_db(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let books = query_books(
        &db,
        "SELECT b.id, b.title, b.pubdate, c.text AS synopsis, a.name AS author_name, a.id AS author_id, d.name AS book_file,
        GROUP_CONCAT(d.format) AS formats
        FROM books b
        JOIN books_authors_link ba ON b.id = ba.book
        JOIN authors a ON ba.author = a.id
        LEFT JOIN comments c ON b.id = c.book
        LEFT JOIN data d ON b.id = d.book GROUP BY b.id;",
        params![],
    );
    let books = match books {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    ctx.insert("books", &books);

    render_template(&data.templates, "books.xml.tera", ctx)
}

