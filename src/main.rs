use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer, Responder};
use authorized::Authorized;
use calibre::tags::Entity as Tag;
use calibre::authors::Entity as Author;
use calibre::books::Entity as Book;
use sea_orm::{Database, DatabaseConnection, EntityTrait};
use std::sync::Arc;
use tera::Tera;

#[macro_use]
extern crate lazy_static;

mod appstate;
mod authorized;
mod calibre;
mod config;

use crate::appstate::AppState;

fn render_template(template: &Tera, name: &str, ctx: tera::Context) -> Result<HttpResponse, Error> {
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

#[actix_web::get("/tags")]
async fn tags(data: web::Data<AppState>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
    let mut ctx = tera::Context::new();

    ctx.insert("config", &data.config);
    ctx.insert(
        "base_url",
        &format!(
            "http://{}:{}",
            &data.config.server.ip, &data.config.server.port
        ),
    );

    let db = &*data.db;
    let tags: Vec<calibre::tags::Model> = Tag::find().all(db).await.unwrap();
    ctx.insert("tags", &tags);

    render_template(&data.templates, "tags.xml.tera", ctx)
}

#[actix_web::get("/authors")]
async fn authors(data: web::Data<AppState>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
    let mut ctx = tera::Context::new();

    ctx.insert("config", &data.config);
    ctx.insert(
        "base_url",
        &format!(
            "http://{}:{}",
            &data.config.server.ip, &data.config.server.port
        ),
    );

    let db = &*data.db;
    let authors: Vec<calibre::authors::Model> = Author::find().all(db).await.unwrap();
    ctx.insert("authors", &authors);

    render_template(&data.templates, "authors.xml.tera", ctx)
}

#[actix_web::get("/books")]
async fn books(data: web::Data<AppState>, _auth: Authorized, _req: HttpRequest) -> impl Responder {
    let mut ctx = tera::Context::new();

    ctx.insert("config", &data.config);
    ctx.insert(
        "base_url",
        &format!(
            "http://{}:{}",
            &data.config.server.ip, &data.config.server.port
        ),
    );

    let db = &*data.db;
    let books: Vec<calibre::books::Model> = Book::find().all(db).await.unwrap();
    ctx.insert("books", &books);

    render_template(&data.templates, "books.xml.tera", ctx)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let config = config::get();
    let ip = config.server.ip.clone();
    let port = config.server.port.clone();
    let templates_path = format!("{}/*", config.server.templates.clone());
    let db_path = format!("sqlite://{}?mode=ro", config.calibre.db_path.clone());

    let db: DatabaseConnection = Database::connect(db_path).await.unwrap();
    let templates = Tera::new(&templates_path).unwrap();

    let state = AppState {
        templates,
        config: config.clone(),
        db: Arc::new(db),
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
    cfg.service(books);
}
