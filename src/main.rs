use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web::http::header::{ContentDisposition, DispositionType};
use actix_files as fs;
use authorized::Authorized;
use std::sync::{Arc, Mutex};
use serde_derive::Serialize;
use tera::Tera;
use rusqlite::{params, Connection}; // Result?

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
    synopsis: Option<String>,
    author_id: i32,
    author_name: String,
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
async fn cover(data: web::Data<AppState>, image_id: web::Path<i32>, _auth: Authorized, _req: HttpRequest) -> Result<fs::NamedFile, Error> {

    let db_lock = data.db.lock().unwrap();
    let image_id = image_id.into_inner();

    let mut stmt = db_lock.prepare(
        "SELECT books.path FROM books WHERE books.id = ?1 AND books.has_cover = true;"
    ).expect("Error preparing SQL statement");

    let path: String = stmt.query_row(rusqlite::params![image_id], |row| {
        row.get(0)
    }).expect("Error retrieving image path");

    let cover_path = format!("{}/{}/cover.jpg", data.config.calibre.library_path, path);

    let file = fs::NamedFile::open(&cover_path)
        .map_err(|e| actix_web::error::ErrorInternalServerError(e))?;
    // let file = fs::NamedFile::open(cover_path)?;
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
    ctx.insert(
        "base_url",
        &format!(
            "http://{}:{}",
            &data.config.server.ip, &data.config.server.port
        ),
    );
    render_template(&data.templates, "index.xml.tera", ctx)
}

//#[actix_web::get("/tags")]
//async fn gettags(data: web::Data<AppState>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
//
//    let mut ctx = tera::Context::new();
//
//    ctx.insert("config", &data.config);
//    ctx.insert(
//        "base_url",
//        &format!(
//            "http://{}:{}",
//            &data.config.server.ip, &data.config.server.port
//        ),
//    );
//
//    let db = &*data.db;
//    let tags: Vec<calibre::tags::Model> = Tag::Entity::find().all(db).await.unwrap();
//    ctx.insert("tags", &tags);
//    render_template(&data.templates, "tags.xml.tera", ctx)
//}

//#[actix_web::get("/tags/{id}")]
//async fn books_by_tag(data: web::Data<AppState>, tag_id: web::Path<i32>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
//
//    let tag_id = tag_id.into_inner();
//    let mut ctx = tera::Context::new();
//
//    ctx.insert("config", &data.config);
//    ctx.insert(
//        "base_url",
//        &format!(
//            "http://{}:{}",
//            &data.config.server.ip, &data.config.server.port
//        ),
//    );
//
//    let db = &*data.db;
//
//
//    //let books_by_tag: Vec<calibre::books::Model> = Book::find().all(db).await.unwrap();
//    ctx.insert("books", &books_by_tag);
//    render_template(&data.templates, "books.xml.tera", ctx)
//}

//#[actix_web::get("/authors")]
//async fn authors(data: web::Data<AppState>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
//    let mut ctx = tera::Context::new();
//
//    ctx.insert("config", &data.config);
//    ctx.insert(
//        "base_url",
//        &format!(
//            "http://{}:{}",
//            &data.config.server.ip, &data.config.server.port
//        ),
//    );
//
//    let db = &*data.db;
//    let authors: Vec<calibre::authors::Model> = Author::find().all(db).await.unwrap();
//    ctx.insert("authors", &authors);
//
//    render_template(&data.templates, "authors.xml.tera", ctx)
//}

#[actix_web::get("/books")]
async fn getbooks(data: web::Data<AppState>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
    let mut ctx = tera::Context::new();

    ctx.insert("config", &data.config);
    ctx.insert(
        "base_url",
        &format!(
            "http://{}:{}",
            &data.config.server.ip, &data.config.server.port
        ),
    );

    let db_lock = data.db.lock().unwrap();
    // let db = &*data.db;
    let mut stmt = db_lock.prepare(
        "SELECT b.id, b.title, b.pubdate, c.text AS synopsis, a.name AS author_name, a.id AS author_id
        FROM books b
        JOIN books_authors_link ba ON b.id = ba.book
        JOIN authors a ON ba.author = a.id
        LEFT JOIN comments c ON b.id = c.book"
    ).expect("Error preparing statement");

    let books_iter = stmt.query_map(params![], |row| {
        Ok(Book {
            id: row.get(0)?,
            title: row.get(1)?,
            pubdate: row.get(2)?,
            synopsis: row.get(3)?,
            author_name: row.get(4)?,
            author_id: row.get(5)?,
        })
    }).expect("Error querying books");
    let books: Vec<Book> = books_iter.map(|b| b.unwrap()).collect();
    // let books: Vec<calibre::books::Model> = Book::find().all(db).await.unwrap();
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
    let templates = Tera::new(&templates_path).unwrap();

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
    // cfg.service(gettags);
    // cfg.service(authors);
    cfg.service(getbooks);
    cfg.service(cover);
    // cfg.service(books_by_tag);
}
