
pub mod templates;
pub mod appstate;
pub mod authorized;
pub mod config;
pub mod tls;
pub mod hash;
pub mod routes;

use actix_web::{web, App, HttpServer};
use rusqlite::Connection;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tera::Result as TeraResult;
use tera::Tera;
use tera::Value;

use config::{Config, Protocol};
use templates::Template;
use routes::{authors, book_file, books_by_author, books_by_tag, cover, getbooks, index, opds, tags};
use appstate::AppState;

// Tera filter to convert format to mime type
fn format_to_mime_filter(value: &Value, _: &HashMap<String, Value>) -> TeraResult<Value> {
    let format_str = value
        .as_str()
        .ok_or_else(|| tera::Error::msg("Expected a string as input for format_to_mime"))?;

    let mime_type = match format_str {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "mobi" => "application/x-mobipocket-ebook",
        _ => "application/octet-stream",
    };

    Ok(json!(mime_type))
}

pub fn create_app(config: &'static Config) -> AppState {

    let mut db_map: HashMap<String, Arc<Mutex<Connection>>> = HashMap::new();
    for (library, path) in &config.calibre.libraries {
        let db_path = format!("{}/metadata.db", path);
        match Connection::open(db_path) {
            Ok(db) => {
                println!("Connected to {}", library);
                db_map.insert(library.clone(), Arc::new(Mutex::new(db)));
            }
            Err(e) => eprintln!("Couldn't connect to {}: {}", library, e),
        };
    }
    if db_map.is_empty() {
        eprintln!("Could not connect to any library");
        std::process::exit(1);
    }

    let mut tera = Tera::default();

    let templates: Vec<(String, String)> = Template::iter()
        .map(|file| {
            let content = Template::get(&file).unwrap();
            let template_str = std::str::from_utf8(content.data.as_ref()).expect("Invalid UTF-8 in template");
            (file.to_string(), template_str.to_string())
        })
        .collect();

    tera.add_raw_templates(templates).expect("Failed to add templates");
    tera.register_filter("format_to_mime", format_to_mime_filter);

    AppState {
        templates: tera,
        config,
        db: db_map,
    }
}

pub async fn run_server(state: AppState) -> std::io::Result<()> {
    let ip = state.config.server.ip.clone();
    let port = state.config.server.port;
    let protocol = state.config.server.protocol.clone();

    match protocol {
        Protocol::Http => {
            println!("Starting HTTP server on {ip}:{port}");

            HttpServer::new(move || {
                App::new()
                    .app_data(web::Data::new(state.clone()))
                    .configure(init)
            })
            .bind((ip, port))?
            .run()
            .await
        }
        Protocol::Https { cert, key } => {
            println!("Starting HTTPS server on {ip}:{port}");

            let config = tls::load_rustls_config(cert.as_str(), key.as_str()).unwrap_or_else(|e| {
                eprintln!("Failed to load TLS config: {}", e);
                std::process::exit(1);
            });

            HttpServer::new(move || {
                App::new()
                    .app_data(web::Data::new(state.clone()))
                    .configure(init)
            })
            .bind_rustls_0_23((ip, port), config)?
            .run()
            .await
        }
    }
}

pub fn init(cfg: &mut web::ServiceConfig) {
    cfg.service(index);
    cfg.service(opds);
    cfg.service(tags);
    cfg.service(authors);
    cfg.service(getbooks);
    cfg.service(book_file);
    cfg.service(cover);
    cfg.service(books_by_tag);
    cfg.service(books_by_author);
}
