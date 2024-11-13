use actix_files as fs;
use actix_web::http::header::{ContentDisposition, DispositionType};
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer, Responder};
use authorized::Authorized;
use rusqlite::{params, Connection, Row};
use serde_derive::Serialize;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tera::Tera;
use tera::Result as TeraResult;
use tera::Value;
use serde_json::json;
use html2text::from_read;

#[macro_use]
extern crate lazy_static;

mod appstate;
mod authorized;
mod config;

use crate::appstate::AppState;

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
    MOBI
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

// Tera filter to convert format to mime type
fn format_to_mime_filter(value: &Value, _: &HashMap<String, Value>) -> TeraResult<Value> {
    let format_str = value.as_str().ok_or_else(|| {
        tera::Error::msg("Expected a string as input for format_to_mime")
    })?;

    let mime_type = match format_str {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "mobi" => "application/x-mobipocket-ebook",
        _ => "application/octet-stream",
    };

    Ok(json!(mime_type))
}

fn render_template(template: &Tera, name: &str, ctx: tera::Context) -> impl Responder {
    match template.render(name, &ctx) {
        Ok(body) => Ok::<_, Error>(
            HttpResponse::Ok()
                .content_type("application/atom+xml")
                .body(body),
        ),
        Err(e) => {
            eprintln!("Template rendering error: {}", e);
            Ok(HttpResponse::InternalServerError()
                .content_type("application/atom+xml")
                .body("Template rendering error"))
        }
    }
}

#[actix_web::get("/cover/{id}")]
async fn cover(
    data: web::Data<AppState>,
    image_id: web::Path<i32>,
    _auth: Authorized,
    _req: HttpRequest,
) -> Result<fs::NamedFile, Error> {
    let db_lock = data.db.lock().unwrap();
    let image_id = image_id.into_inner();

    let mut stmt = db_lock
        .prepare("SELECT books.path FROM books WHERE books.id = ?1 AND books.has_cover = true;")
        .expect("Error preparing SQL statement");

    let path: String = stmt
        .query_row(rusqlite::params![image_id], |row| row.get(0))
        .expect("Error retrieving image path");

    let cover_path = format!("{}/{}/cover.jpg", data.config.calibre.library_path, path);

    let file = fs::NamedFile::open(&cover_path)
        .map_err(|e| actix_web::error::ErrorInternalServerError(e))?;

    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        }))
}

#[actix_web::get("/file/{id}/{format}")]
async fn book_file(
    data: web::Data<AppState>,
    path: web::Path<(i32, String)>,
    _auth: Authorized,
    _req: HttpRequest,
) -> Result<fs::NamedFile, Error> {
    let db_lock = data.db.lock().unwrap();
    let (id, format) = path.into_inner();

    let mut stmt = db_lock
        .prepare("SELECT b.path, d.name AS file
                  FROM books b
                  LEFT JOIN data d ON b.id = d.book
                  WHERE b.id = ?1 GROUP BY b.id;")
        .expect("Error preparing SQL statement");

    let row_mapper = |row: &Row| -> rusqlite::Result<(String, String)> {
        let path: String = row.get(0)?;
        let file: String = row.get(1)?;
        Ok((path, file))
    };

    let (path, file): (String, String) = stmt.query_row(rusqlite::params![id], row_mapper)
                                            .expect("Error retrieving file path from database");

    let book_file_path = format!("{}/{}/{}.{}", data.config.calibre.library_path, path, file, format);

    let file = fs::NamedFile::open(&book_file_path)
        .map_err(|e| actix_web::error::ErrorInternalServerError(e))?;

    Ok(file
        .use_last_modified(true)
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![],
        }))
}

#[actix_web::get("/opds")]
async fn opds(data: web::Data<AppState>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
    let mut ctx = tera::Context::new();

    ctx.insert("config", &data.config);
    render_template(&data.templates, "index.xml.tera", ctx)
}

#[actix_web::get("/tags")]
async fn tags(data: web::Data<AppState>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
    let mut ctx = tera::Context::new();

    ctx.insert("config", &data.config);

    let db_lock = data.db.lock().unwrap();

    let mut stmt = db_lock
        .prepare("SELECT id, name FROM tags;")
        .expect("Error preparing statement");

    let books_iter = stmt
        .query_map(params![], |row| {
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .expect("Error querying books");

    let tags: Vec<Tag> = books_iter.map(|b| b.unwrap()).collect();
    ctx.insert("tags", &tags);
    render_template(&data.templates, "tags.xml.tera", ctx)
}

#[actix_web::get("/tags/{id}")]
async fn books_by_tag(
    data: web::Data<AppState>,
    tag_id: web::Path<i32>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let tag_id = tag_id.into_inner();
    let mut ctx = tera::Context::new();
    ctx.insert("config", &data.config);

    let db_lock = data.db.lock().unwrap();
    let mut stmt = db_lock
        .prepare(
            "SELECT b.id, b.title, b.pubdate, c.text AS synopsis, a.name AS author_name, a.id AS author_id, d.name AS book_file,
            GROUP_CONCAT(d.format) AS formats
            FROM books b
            JOIN books_tags_link bt ON b.id = bt.book
            JOIN tags t ON bt.tag = t.id
            JOIN books_authors_link ba ON b.id = ba.book
            JOIN authors a ON ba.author = a.id
            LEFT JOIN comments c ON b.id = c.book
            LEFT JOIN data d ON b.id = d.book
            WHERE t.id = ?1 GROUP BY b.id;")
        .expect("Error preparing SQL statement");

    let books_iter = stmt
        .query_map(params![tag_id], |row| {
            let synopsis = row.get(3).unwrap_or("".to_string());
            let synopsis = from_read(synopsis.as_bytes(), 100).unwrap();
            let format_str = row.get("formats").unwrap_or("".to_string());
            let formats = format_str
                .split(',')
                .filter_map(Format::from_str)
                .collect();
            Ok(Book {
                id: row.get(0)?,
                title: row.get(1)?,
                pubdate: row.get(2)?,
                synopsis,
                author_name: row.get(4)?,
                author_id: row.get(5)?,
                book_file: row.get(6)?,
                formats
            })
        })
        .expect("Error querying books");


    let books_by_tag: Vec<Book> = books_iter.map(|b| b.unwrap()).collect();

    ctx.insert("books", &books_by_tag);
    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::get("/authors")]
async fn authors(
    data: web::Data<AppState>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let mut ctx = tera::Context::new();

    ctx.insert("config", &data.config);

    let db_lock = data.db.lock().unwrap();

    let mut stmt = db_lock
        .prepare("SELECT id, name FROM authors;")
        .expect("Error preparing statement");

    let author_iter = stmt
        .query_map(params![], |row| {
            Ok(Author {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .expect("Error querying authors");

    let authors: Vec<Author> = author_iter.map(|b| b.unwrap()).collect();
    ctx.insert("authors", &authors);

    render_template(&data.templates, "authors.xml.tera", ctx)
}

#[actix_web::get("/authors/{id}")]
async fn books_by_author(
    data: web::Data<AppState>,
    author_id: web::Path<i32>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let author_id = author_id.into_inner();
    let mut ctx = tera::Context::new();
    ctx.insert("config", &data.config);

    let db_lock = data.db.lock().unwrap();
    let mut stmt = db_lock
        .prepare(
            "SELECT b.id, b.title, b.pubdate, c.text AS synopsis, a.name AS author_name, a.id AS author_id, d.name AS book_file,
            GROUP_CONCAT(d.format) AS formats
            FROM books b
            JOIN books_tags_link bt ON b.id = bt.book
            JOIN tags t ON bt.tag = t.id
            JOIN books_authors_link ba ON b.id = ba.book
            JOIN authors a ON ba.author = a.id
            LEFT JOIN comments c ON b.id = c.book
            LEFT JOIN data d ON b.id = d.book
            WHERE a.id = ?1 GROUP BY b.id;")
        .expect("Error preparing SQL statement");

    let books_iter = stmt
        .query_map(params![author_id], |row| {
            let synopsis = row.get(3).unwrap_or("".to_string());
            let synopsis = from_read(synopsis.as_bytes(), 100).unwrap();
            let format_str = row.get("formats").unwrap_or("".to_string());
            let formats = format_str
                .split(',')
                .filter_map(Format::from_str)
                .collect();
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
        })
        .expect("Error querying books");

    let books_by_author: Vec<Book> = books_iter.map(|b| b.unwrap()).collect();

    ctx.insert("books", &books_by_author);
    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::get("/books")]
async fn getbooks(
    data: web::Data<AppState>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let mut ctx = tera::Context::new();

    ctx.insert("config", &data.config);

    let db_lock = data.db.lock().unwrap();

    let mut stmt = db_lock.prepare(
        "SELECT b.id, b.title, b.pubdate, c.text AS synopsis, a.name AS author_name, a.id AS author_id, d.name AS book_file,
        GROUP_CONCAT(d.format) AS formats
        FROM books b
        JOIN books_authors_link ba ON b.id = ba.book
        JOIN authors a ON ba.author = a.id
        LEFT JOIN comments c ON b.id = c.book
        LEFT JOIN data d ON b.id = d.book GROUP BY b.id;",
    ).expect("Error preparing statement");

    let books_iter = stmt
        .query_map(params![], |row| {
            let synopsis = row.get(3).unwrap_or("".to_string());
            let synopsis = from_read(synopsis.as_bytes(), 100).unwrap();
            let format_str = row.get("formats").unwrap_or("".to_string());
            let formats = format_str
                .split(',')
                .filter_map(Format::from_str)
                .collect();
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
        })
        .expect("Error querying books");
    let books: Vec<Book> = books_iter.map(|b| b.unwrap()).collect();

    ctx.insert("books", &books);

    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let config = config::get();
    let ip = config.server.ip.clone();
    let port = config.server.port.clone();
    let templates_path = format!("{}/*", config.server.templates.clone());
    let db_path = format!("{}/metadata.db", config.calibre.library_path);
    let db = Connection::open(db_path).unwrap();
    let mut templates = Tera::new(&templates_path).unwrap();
    templates.register_filter("format_to_mime", format_to_mime_filter);

    let state = AppState {
        templates,
        config: config.clone(),
        db: Arc::new(Mutex::new(db)),
    };

    println!("Starting server on {ip}:{port}");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(init)
    })
    .bind((ip, port))?
    .run()
    .await
}

fn init(cfg: &mut web::ServiceConfig) {
    cfg.service(opds);
    cfg.service(tags);
    cfg.service(authors);
    cfg.service(getbooks);
    cfg.service(book_file);
    cfg.service(cover);
    cfg.service(books_by_tag);
    cfg.service(books_by_author);
}
