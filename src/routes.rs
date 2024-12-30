
use actix_web::{web, Error, HttpRequest, HttpResponse, Responder};
use actix_web::http::header::{ContentDisposition, DispositionType};
use actix_files as fs;
use tera::Tera;
use serde_derive::Serialize;
use html2text::from_read;
use rusqlite::{params, Row};
use crate::authorized::Authorized;
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
        Some(db) => db.lock().unwrap(),
        None => {
            return Err(actix_web::error::ErrorInternalServerError(
                "Database not found",
            ))
        }
    };
    let library_path = data.config.calibre.libraries.get(&lib).unwrap();

    let mut stmt = db_lock
        .prepare("SELECT books.path FROM books WHERE books.id = ?1 AND books.has_cover = true;")
        .expect("Error preparing SQL statement");

    let path: String = stmt
        .query_row(rusqlite::params![image_id], |row| row.get(0))
        .expect("Error retrieving image path");

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
        Some(db) => db.lock().unwrap(),
        None => {
            return Err(actix_web::error::ErrorInternalServerError(
                "Database not found",
            ))
        }
    };
    let library_path = data.config.calibre.libraries.get(&db).unwrap();

    let mut stmt = db_lock
        .prepare(
            "SELECT b.path, d.name AS file
                  FROM books b
                  LEFT JOIN data d ON b.id = d.book
                  WHERE b.id = ?1 GROUP BY b.id;",
        )
        .expect("Error preparing SQL statement");

    let row_mapper = |row: &Row| -> rusqlite::Result<(String, String)> {
        let path: String = row.get(0)?;
        let file: String = row.get(1)?;
        Ok((path, file))
    };

    let (path, file): (String, String) = stmt
        .query_row(rusqlite::params![id], row_mapper)
        .expect("Error retrieving file path from database");

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
async fn index(data: web::Data<AppState>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
    let mut ctx = tera::Context::new();

    let libraries: Vec<String> = data.db.keys().cloned().collect();

    if libraries.len() == 1 {
        let lib = &libraries[0];
        return HttpResponse::Found()
            .append_header(("Location", format!("/{}", lib)))
            .finish();
    }

    ctx.insert("config", &data.config);
    ctx.insert("libraries", &libraries);
    render_template(&data.templates, "index.xml.tera", ctx)
}

#[actix_web::get("/{lib}")]
async fn opds(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let mut ctx = tera::Context::new();
    let lib = path.into_inner();

    ctx.insert("config", &data.config);
    ctx.insert("lib", &lib);
    render_template(&data.templates, "opds.xml.tera", ctx)
}

#[actix_web::get("/{lib}/tags")]
async fn tags(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let mut ctx = tera::Context::new();
    let lib = path.into_inner();
    let db = match data.db.get(&lib) {
        Some(db) => db.lock().unwrap(),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    ctx.insert("config", &data.config);
    ctx.insert("lib", &lib);

    let mut stmt = db
        .prepare("SELECT id, name FROM tags;")
        .expect("Error preparing statement");

    let tags_iter = stmt
        .query_map(params![], |row| {
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .expect("Error querying tags");

    let tags: Vec<Tag> = tags_iter.map(|t| t.unwrap()).collect();
    ctx.insert("tags", &tags);
    render_template(&data.templates, "tags.xml.tera", ctx)
}

#[actix_web::get("{lib}/tags/{id}")]
async fn books_by_tag(
    data: web::Data<AppState>,
    path: web::Path<(String, i32)>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let (lib, tag_id) = path.into_inner();
    let mut ctx = tera::Context::new();
    ctx.insert("config", &data.config);
    ctx.insert("lib", &lib);

    let db = match data.db.get(&lib) {
        Some(db) => db.lock().unwrap(),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let mut stmt = db
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
        })
        .expect("Error querying books");

    let books_by_tag: Vec<Book> = books_iter.map(|b| b.unwrap()).collect();

    ctx.insert("books", &books_by_tag);
    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::get("{lib}/authors")]
async fn authors(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let mut ctx = tera::Context::new();
    let lib = path.into_inner();

    ctx.insert("config", &data.config);
    ctx.insert("lib", &lib);

    let db = match data.db.get(&lib) {
        Some(db) => db.lock().unwrap(),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let mut stmt = db
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

#[actix_web::get("{lib}/authors/{id}")]
async fn books_by_author(
    data: web::Data<AppState>,
    author_id: web::Path<(String, i32)>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let (lib, author_id) = author_id.into_inner();
    let mut ctx = tera::Context::new();
    ctx.insert("config", &data.config);
    ctx.insert("lib", &lib);

    let db = match data.db.get(&lib) {
        Some(db) => db.lock().unwrap(),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let mut stmt = db
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
        })
        .expect("Error querying books");

    let books_by_author: Vec<Book> = books_iter.map(|b| b.unwrap()).collect();

    ctx.insert("books", &books_by_author);
    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::get("{lib}/books")]
async fn getbooks(
    data: web::Data<AppState>,
    path: web::Path<String>,
    _auth: Authorized,
    _req: HttpRequest,
) -> impl Responder {
    let mut ctx = tera::Context::new();
    let lib = path.into_inner();

    ctx.insert("config", &data.config);
    ctx.insert("lib", &lib);

    let db = match data.db.get(&lib) {
        Some(db) => db.lock().unwrap(),
        None => return HttpResponse::NotFound().body(format!("Database '{}' not found", lib)),
    };

    let mut stmt = db.prepare(
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
        })
        .expect("Error querying books");
    let books: Vec<Book> = books_iter.map(|b| b.unwrap()).collect();

    ctx.insert("books", &books);

    render_template(&data.templates, "books.xml.tera", ctx)
}

