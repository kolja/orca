use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Error, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::sync::Arc;
use tera::Tera;
// use std::process::exit;

#[macro_use]
extern crate lazy_static;

mod calibre;
mod config;

use sea_orm::{Database, DatabaseConnection, EntityTrait};

use calibre::tags::Entity as Tag;

#[derive(Clone)]
struct AppState {
    templates: tera::Tera,
    db: Arc<DatabaseConnection>,
}

fn authorized(req: HttpRequest) -> Option<String> {
    let credentials = req
        .headers()
        .get("authorization")
        .and_then(|s| s.to_str().ok()?.strip_prefix("Basic "))
        .and_then(|s| BASE64.decode(s).ok())
        .and_then(|vec| String::from_utf8(vec).ok())?;

    // at this point we have the credentials in the form of "username:password"
    // and we could do some propper OAuth2 validation.

    let config = config::get();

    if config.authentication.credentials.contains(&credentials) {
        Some(credentials)
    } else {
        None
    }
}

#[actix_web::get("/opds")]
async fn opds(data: web::Data<AppState>, req: HttpRequest) -> Result<HttpResponse, Error> {
    let template = &data.templates;
    let ctx = tera::Context::new();
    match authorized(req) {
        Some(_credentials) => {

            match template.render("index.xml.tera", &ctx) {
                Ok(body) => Ok(HttpResponse::Ok()
                    .content_type("application/atom+xml")
                    .body(body)),
                Err(e) => {
                    eprintln!("Template rendering error: {}", e);
                    Ok(HttpResponse::InternalServerError()
                        .content_type("application/atom+xml")
                        .body("Template rendering error"))
                }
            }

        },
        None => Ok(HttpResponse::Unauthorized()
            .insert_header(("WWW-Authenticate", "Basic realm=\"Login Required\""))
            .body("Unauthorized")),
    }
}

#[actix_web::get("/tags")]
async fn tags(data: web::Data<AppState>, req: HttpRequest) -> Result<HttpResponse, Error> {
    let template = &data.templates;
    let mut ctx = tera::Context::new();
    match authorized(req) {
        Some(credentials) => {

            let db = &*data.db;
            println!("Authorized: {}", credentials);
            println!("db: {:?}", data.db);

            let tags: Vec<calibre::tags::Model> = Tag::find().all(db).await.unwrap();
            ctx.insert("tags", &tags);

            match template.render("tags.xml.tera", &ctx) {
                Ok(body) => Ok(HttpResponse::Ok()
                    .content_type("application/atom+xml")
                    .body(body)),
                Err(e) => {
                    eprintln!("Template rendering error: {}", e);
                    Ok(HttpResponse::InternalServerError()
                        .content_type("application/atom+xml")
                        .body("Template rendering error"))
                }
            }

        },
        None => Ok(HttpResponse::Unauthorized()
            .insert_header(("WWW-Authenticate", "Basic realm=\"Login Required\""))
            .body("Unauthorized")),
    }
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
        db: Arc::new(db),
    };

    println!("Starting server on {ip}:{port}");

    HttpServer::new(move ||
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(init)
        )
        .bind((ip, port))?
        .run()
        .await
}

fn init(cfg: &mut web::ServiceConfig) {
    cfg.service(opds);
    cfg.service(tags);
}
