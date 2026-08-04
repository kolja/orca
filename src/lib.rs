
pub mod templates;
pub mod appstate;
pub mod authorized;
pub mod calibre;
pub mod config;
pub mod tls;
pub mod hash;
pub mod opds2;
pub mod routes;
pub mod routes_v2;
pub mod pattern;

use actix_web::{web, App, HttpServer};
use anyhow::{anyhow, Result};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tera::{Kwargs, State, Tera};

use config::{Config, Protocol};
use templates::Template;
use routes::{health, authors, book_file, books_by_author, books_by_tag, cover, getbooks, index, opds, recently_added, tags};
use appstate::AppState;

// Tera filter to convert format to mime type. The table itself lives next to the
// formats it names, so the OPDS 2.0 links can use it too.
fn format_to_mime_filter(format: &str, _: Kwargs, _: &State) -> &'static str {
    calibre::mime(format)
}

/// Open one Calibre library, or explain why it cannot be served.
fn open_library(library: &str, path: &str) -> Result<Connection> {
    let db_path = format!("{}/metadata.db", path);

    // Checked before opening. Otherwise `Connection::open` would *create* an empty database
    if !Path::new(&db_path).is_file() {
        return Err(anyhow!("library '{}': no Calibre database at '{}'", library, db_path));
    }

    let db = Connection::open(&db_path)
        .map_err(|e| anyhow!("library '{}': could not open '{}': {}", library, db_path, e))?;

    // Opening succeeds for anything SQLite can read, so only a query proves that
    // the file behind a configured path is really a Calibre library.
    db.query_row("SELECT COUNT(*) FROM books;", [], |row| row.get::<_, i64>(0))
        .map_err(|e| anyhow!("library '{}': '{}' is not a Calibre database: {}", library, db_path, e))?;

    Ok(db)
}

/// Path segments reserved to orca. Can't serve a library under these.
const RESERVED: [&str; 2] = ["v2", "health"];

pub fn create_app(config: &'static Config) -> Result<AppState> {

    if config.calibre.libraries.is_empty() {
        return Err(anyhow!("no libraries configured under [calibre.libraries]"));
    }

    // A library named after one of Orca's own routes would be unreachable
    if let Some(library) = config.calibre.libraries.keys().find(|name| RESERVED.contains(&name.as_str())) {
        return Err(anyhow!("library '{}': the name is reserved by Orca itself", library));
    }

    // Every configured library has to open
    let mut db_map: HashMap<String, Arc<Mutex<Connection>>> = HashMap::new();
    for (library, settings) in &config.calibre.libraries {
        let db = open_library(library, &settings.path)?;
        println!("Connected to {}", library);
        db_map.insert(library.clone(), Arc::new(Mutex::new(db)));
    }

    let mut tera = Tera::default();

    // Tera resolves filters when a template is added, so custom filters have to
    // be registered before `add_raw_templates`.
    tera.register_filter("format_to_mime", format_to_mime_filter);
    // Embedded template names end in `.xml.tera`, which Tera does not
    // auto-escape by default. Escape dynamic values so Calibre metadata such
    // as ampersands cannot produce malformed OPDS XML.
    tera.autoescape_on(vec![".html", ".htm", ".xml", ".xml.tera"]);

    let templates: Vec<(String, String)> = Template::iter()
        .map(|file| {
            let content = Template::get(&file).unwrap();
            let template_str = std::str::from_utf8(content.data.as_ref()).expect("Invalid UTF-8 in template");
            (file.to_string(), template_str.to_string())
        })
        .collect();

    tera.add_raw_templates(templates).expect("Failed to add templates");

    Ok(AppState {
        templates: tera,
        config,
        db: db_map,
    })
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
    cfg.service(health);

    cfg.service(routes_v2::catalog);
    cfg.service(routes_v2::library_root);
    cfg.service(routes_v2::all_books);
    cfg.service(routes_v2::recently_added);
    cfg.service(routes_v2::single_book);

    cfg.service(index);
    cfg.service(opds);
    cfg.service(tags);
    cfg.service(authors);
    cfg.service(getbooks);
    cfg.service(recently_added);
    cfg.service(book_file);
    cfg.service(cover);
    cfg.service(books_by_tag);
    cfg.service(books_by_author);
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Authentication, Calibre, Catalog, Library, Protocol, Server};
    use std::fs;
    use tempfile::TempDir;

    // `create_app` takes a `&'static Config`; leaking one per test is cheap and
    // keeps these tests independent of any config file on disk.
    fn config_for(libraries: &[(&str, &str)]) -> &'static Config {
        Box::leak(Box::new(Config {
            server: Server {
                ip: "127.0.0.1".to_string(),
                port: 8080,
                public_url: None,
                protocol: Protocol::Http,
            },
            authentication: Authentication::default(),
            calibre: Calibre {
                libraries: libraries
                    .iter()
                    .map(|(name, path)| {
                        (
                            name.to_string(),
                            Library { path: path.to_string(), author: None },
                        )
                    })
                    .collect(),
            },
            catalog: Catalog::default(),
        }))
    }

    // `expect_err` is unavailable here: it needs `AppState: Debug`, which would
    // pull a Debug bound through Tera and Config for no benefit.
    fn refusal(result: Result<AppState>, why: &str) -> String {
        match result {
            Ok(_) => panic!("{}", why),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn opens_a_real_calibre_library() {
        assert!(create_app(config_for(&[("library", "tests/calibre")])).is_ok());
    }

    #[test]
    fn a_mistyped_path_refuses_to_start() {
        let err = refusal(
            create_app(config_for(&[("library", "tests/calibr")])),
            "a library that is not there must not start a server",
        );
        assert!(err.contains("no Calibre database at 'tests/calibr/metadata.db'"), "{}", err);
    }

    // The reason `open_library` checks before opening: SQLite would create the missing file, 
    // turning a config typo into a 500 on every request instead of an error at startup.
    #[test]
    fn a_mistyped_path_does_not_create_a_database() {
        let dir = TempDir::new().unwrap();

        assert!(create_app(config_for(&[("library", dir.path().to_str().unwrap())])).is_err());
        assert!(!dir.path().join("metadata.db").exists());
    }

    #[test]
    fn a_file_that_is_not_a_calibre_database_refuses_to_start() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("metadata.db"), b"not a database at all").unwrap();

        let err = refusal(
            create_app(config_for(&[("library", dir.path().to_str().unwrap())])),
            "a file that is not a Calibre database must not start a server",
        );
        assert!(err.contains("is not a Calibre database"), "{}", err);
    }

    // One working library is no excuse to skip a broken one: the catalog would
    // silently serve half the books it is configured for.
    #[test]
    fn a_single_broken_library_refuses_to_start() {
        let err = refusal(
            create_app(config_for(&[("good", "tests/calibre"), ("bad", "tests/nope")])),
            "a broken library must not be skipped",
        );
        assert!(err.contains("library 'bad'"), "{}", err);
    }

    #[test]
    fn no_libraries_refuses_to_start() {
        let err = refusal(
            create_app(config_for(&[])),
            "a server with no libraries has nothing to serve",
        );
        assert!(err.contains("no libraries configured"), "{}", err);
    }
}
