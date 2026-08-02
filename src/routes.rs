
use actix_web::{web, Error, HttpRequest, HttpResponse, Responder};
use actix_web::http::header::{ContentDisposition, DispositionType};
use actix_files as fs;
use tera::Tera;
use serde_derive::Serialize;
use html2text::from_read;
use rusqlite::{params, params_from_iter, Connection, Row};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use crate::authorized::Authorized;
use crate::appstate::AppState;
use crate::config::Config;

#[derive(Debug, Serialize)]
struct Book {
    id: i32,
    uuid: String,
    title: String,
    pubdate: String,
    updated: String,
    synopsis: String,
    formats: Vec<Format>,
    authors: Vec<Author>,
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

/// Calibre timestamps look like this: `2024-12-30 14:13:52.213388+00:00`.
/// According to RFC 4287 §3.3 date and time must be separated by 'T'
fn to_rfc3339(calibre: &str) -> String {
    calibre.replacen(' ', "T", 1)
}

fn library_updated(db: &Connection) -> String {
    let latest: Option<String> = db
        .query_row("SELECT MAX(last_modified) FROM books;", params![], |row| {
            row.get(0)
        })
        .unwrap_or(None);
    // Calibre's own default for a book that has never been touched.
    to_rfc3339(&latest.unwrap_or_else(|| "2000-01-01 00:00:00+00:00".to_string()))
}

fn feed_id(path: &str) -> String {
    match path.trim_matches('/') {
        "" => "urn:orca:root".to_string(),
        path => format!("urn:orca:{}", path.replace('/', ":")),
    }
}

/// config plus the absolute URL of this feed for `<link rel="self">`.
/// Some clients resolve the relative hrefs of entries against the self link
fn feed_ctx(req: &HttpRequest, config: &Config) -> tera::Context {
    let mut ctx = tera::Context::new();
    let origin = origin(req, config);
    ctx.insert("self_url", &format!("{}{}", origin, req.path()));
    ctx.insert("base", &origin);
    ctx.insert("feed_id", &feed_id(req.path()));
    ctx.insert("config", config);
    ctx
}

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

/// The columns every book feed needs
const BOOK_COLUMNS: &str = "b.id, b.uuid, b.title, b.pubdate, b.last_modified, c.text AS synopsis,
    (SELECT GROUP_CONCAT(format) FROM data WHERE book = b.id) AS formats";

/// Run one of the book queries and map its rows to `Book`s.
fn query_books(
    db: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<Book>> {
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        let synopsis: String = row.get("synopsis").unwrap_or_default();
        let synopsis = from_read(synopsis.as_bytes(), 100).unwrap_or_default();
        let format_str: String = row.get("formats").unwrap_or_default();
        let formats = format_str.split(',').filter_map(Format::from_str).collect();
        Ok(Book {
            id: row.get("id")?,
            uuid: row.get("uuid").unwrap_or_default(),
            title: row.get("title")?,
            pubdate: to_rfc3339(&row.get::<_, String>("pubdate")?),
            updated: to_rfc3339(&row.get::<_, String>("last_modified")?),
            synopsis,
            formats,
            authors: Vec::new(),
        })
    })?;

    let mut books = collect_rows(rows, "book");
    let book_ids: Vec<i32> = books.iter().map(|book| book.id).collect();
    let mut by_book = authors_by_book(db, &book_ids)?;
    for book in &mut books {
        book.authors = by_book.remove(&book.id).unwrap_or_default();
    }
    Ok(books)
}

/// The authors of each of the given books, by book id. 
/// Books are joined to their authors in a separate query
fn authors_by_book(db: &Connection, book_ids: &[i32]) -> rusqlite::Result<HashMap<i32, Vec<Author>>> {
    if book_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = vec!["?"; book_ids.len()].join(",");
    let mut stmt = db.prepare(&format!(
        "SELECT ba.book, a.id, a.name
            FROM books_authors_link ba
            JOIN authors a ON ba.author = a.id
            WHERE ba.book IN ({})
            ORDER BY ba.book, a.sort;",
        placeholders
    ))?;

    let rows = stmt.query_map(params_from_iter(book_ids), |row| {
        Ok((
            row.get::<_, i32>(0)?,
            Author {
                id: row.get(1)?,
                name: row.get(2)?,
            },
        ))
    })?;

    let mut by_book: HashMap<i32, Vec<Author>> = HashMap::new();
    for (book, author) in collect_rows(rows, "book author") {
        by_book.entry(book).or_default().push(author);
    }
    Ok(by_book)
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

    let updated = data
        .db
        .values()
        .map(|db| library_updated(&lock_db(db)))
        .max()
        .unwrap_or_else(|| "2000-01-01T00:00:00+00:00".to_string());

    let mut ctx = feed_ctx(&req, data.config);
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
        Some(db) => lock_db(db),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let mut ctx = feed_ctx(&req, data.config);
    ctx.insert("lib", &lib);
    ctx.insert("updated", &library_updated(&db));
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
    ctx.insert("updated", &library_updated(&db));
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
        &format!(
            "SELECT {}
                FROM books b
                JOIN books_tags_link bt ON b.id = bt.book
                LEFT JOIN comments c ON b.id = c.book
                WHERE bt.tag = ?1 GROUP BY b.id;",
            BOOK_COLUMNS
        ),
        params![tag_id],
    );
    let books_by_tag = match books_by_tag {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    ctx.insert("books", &books_by_tag);
    ctx.insert("updated", &library_updated(&db));
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
    ctx.insert("updated", &library_updated(&db));

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
        &format!(
            "SELECT {}
                FROM books b
                JOIN books_authors_link ba ON b.id = ba.book
                LEFT JOIN comments c ON b.id = c.book
                WHERE ba.author = ?1 GROUP BY b.id;",
            BOOK_COLUMNS
        ),
        params![author_id],
    );
    let books_by_author = match books_by_author {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    ctx.insert("books", &books_by_author);
    ctx.insert("updated", &library_updated(&db));
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
        &format!(
            "SELECT {}
                FROM books b
                LEFT JOIN comments c ON b.id = c.book;",
            BOOK_COLUMNS
        ),
        params![],
    );
    let books = match books {
        Ok(books) => books,
        Err(e) => return server_error("Error querying books", e),
    };

    ctx.insert("books", &books);
    ctx.insert("updated", &library_updated(&db));

    render_template(&data.templates, "books.xml.tera", ctx)
}

