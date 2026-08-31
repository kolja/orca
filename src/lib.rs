
pub mod templates;
pub mod appstate;
pub mod authdoc;
pub mod authorized;
pub mod calibre;
pub mod config;
pub mod landing;
pub mod tls;
pub mod hash;
pub mod opds2;
pub mod routes;
pub mod routes_v2;
pub mod shelf;
pub mod public;

use actix_web::body::BoxBody;
use actix_web::dev::{ServiceFactory, ServiceRequest, ServiceResponse};
use actix_web::middleware::NormalizePath;
use actix_web::{web, App, HttpServer};
use anyhow::{anyhow, Result};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tera::{Kwargs, State, Tera};

use config::{Config, Protocol};
use landing::Landing;
use templates::Template;
use routes::{health, all_series, authentication, authors, book_file, books_by_author, books_by_publisher, books_by_tag, books_in_series, cover, catalog_moved, feed_moved, getbooks, index, library_moved, logo, named_shelf, opds, opensearch, publishers, recently_added, search, tags};
use appstate::AppState;

// Tera filter to convert format to mime type -- OPDS v2 links can use it too.
fn format_to_mime_filter(format: &str, _: Kwargs, _: &State) -> &'static str {
    calibre::mime(format)
}

/// Open one Calibre library, or explain why it cannot be served.
fn open_library(library: &str, path: &str) -> Result<Connection> {
    let db_path = format!("{}/metadata.db", path);

    // Checked before opening
    if !Path::new(&db_path).is_file() {
        return Err(anyhow!("library '{}': no Calibre database at '{}'", library, db_path));
    }

    // Orca never writes to a Calibre library, so the connection should be read only
    let read_only = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;

    let db = Connection::open_with_flags(&db_path, read_only)
        .map_err(|e| anyhow!("library '{}': could not open '{}': {}", library, db_path, e))?;

    // Opening succeeds for anything SQLite can read, so only a query proves that
    // the file behind a configured path is really a Calibre library.
    db.query_row("SELECT COUNT(*) FROM books;", [], |row| row.get::<_, i64>(0))
        .map_err(|e| anyhow!("library '{}': '{}' is not a Calibre database: {}", library, db_path, e))?;

    Ok(db)
}

/// Path segments reserved to orca. Can't serve a library under these.
pub(crate) const RESERVED: [&str; 4] = ["v1", "v2", "health", "auth"];

fn check_authentication(config: &Config) -> Result<()> {
    let Some(auth) = &config.authentication else {
        println!("No [authentication] section: serving every library without authentication");
        return Ok(());
    };

    if auth.login.is_empty() && auth.public.is_empty() {
        return Err(anyhow!(
            "[authentication] has no logins and names nothing public: no request could be \
             served. Remove the section from your config if you don't want authentication."
        ));
    }

    for rule in &auth.public {
        match rule.library() {
            Some(library) if !config.calibre.libraries.contains_key(library) => {
                return Err(anyhow!(
                    "[authentication] public names '{}', which is not a configured library",
                    library
                ));
            }
            _ => {}
        }
    }

    if auth.login.is_empty() {
        eprintln!(
            "Warning: no logins configured. Everything outside `public` will refuse every request."
        );
    }

    Ok(())
}

/// the server won't start if a page does not parse.
fn landing_pages(config: &'static Config) -> Result<HashMap<String, Arc<Landing>>> {
    let mut pages = HashMap::new();
    // one file shared by several libraries is read once
    let mut read: HashMap<PathBuf, Arc<Landing>> = HashMap::new();

    for library in config.calibre.libraries.keys() {
        let Some(path) = config.landing_page(library) else {
            continue;
        };

        let page = match read.get(&path) {
            Some(page) => page.clone(),
            None => {
                let page = Arc::new(Landing::read(&path)
                    .map_err(|e| anyhow!("library '{}': {}", library, e))?);
                read.insert(path, page.clone());
                page
            }
        };

        pages.insert(library.clone(), page);
    }

    Ok(pages)
}

pub fn create_app(config: &'static Config) -> Result<AppState> {

    if config.calibre.libraries.is_empty() {
        return Err(anyhow!("no libraries configured under [calibre.libraries]"));
    }

    // A library named after one of Orca's own routes would be unreachable
    if let Some(library) = config.calibre.libraries.keys().find(|name| RESERVED.contains(&name.as_str())) {
        return Err(anyhow!("library '{}': the name is reserved by Orca itself", library));
    }

    check_authentication(config)?;

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
        pages: landing_pages(config)?,
    })
}

/// `NormalizePath::trim` rewrites the path before anything else sees it
///  `authorized.rs` reads the path after normalisation
pub fn app(
    state: AppState,
) -> App<
    impl ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse<BoxBody>,
        Error = actix_web::Error,
        InitError = (),
    >,
> {
    App::new()
        .wrap(NormalizePath::trim())
        .app_data(web::Data::new(state))
        .configure(init)
}

pub async fn run_server(state: AppState) -> std::io::Result<()> {
    let ip = state.config.server.ip.clone();
    let port = state.config.server.port;
    let protocol = state.config.server.protocol.clone();

    match protocol {
        Protocol::Http => {
            println!("Starting HTTP server on {ip}:{port}");

            HttpServer::new(move || app(state.clone()))
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

            HttpServer::new(move || app(state.clone()))
            .bind_rustls_0_23((ip, port), config)?
            .run()
            .await
        }
    }
}

pub fn init(cfg: &mut web::ServiceConfig) {
    cfg.service(health);

    // Before `/{lib}`, which would otherwise answer for `/auth` as well.
    cfg.service(authentication);
    cfg.service(logo);

    cfg.service(routes_v2::catalog);
    cfg.service(routes_v2::library_root);
    cfg.service(routes_v2::all_books);
    cfg.service(routes_v2::recently_added);
    cfg.service(routes_v2::single_book);
    cfg.service(routes_v2::authors);
    cfg.service(routes_v2::tags);
    cfg.service(routes_v2::all_series);
    cfg.service(routes_v2::publishers);
    cfg.service(routes_v2::books_by_author);
    cfg.service(routes_v2::books_by_tag);
    cfg.service(routes_v2::books_in_series);
    cfg.service(routes_v2::books_by_publisher);
    cfg.service(routes_v2::named_shelf);
    cfg.service(routes_v2::search);

    // The OPDS 1.2 feeds under `/v1`, JSON is under `/v2`.
    cfg.service(index);
    cfg.service(opds);
    cfg.service(opensearch);
    cfg.service(search);
    cfg.service(tags);
    cfg.service(authors);
    cfg.service(all_series);
    cfg.service(publishers);
    cfg.service(getbooks);
    cfg.service(recently_added);
    cfg.service(named_shelf);
    cfg.service(books_by_tag);
    cfg.service(books_by_author);
    cfg.service(books_in_series);
    cfg.service(books_by_publisher);

    // Bytes rather than OPDS, and version-neutral: both feeds link here.
    // Before the aliases below, which would otherwise redirect every cover.
    cfg.service(book_file);
    cfg.service(cover);

    // Last, because `{lib}` matches anything at all -- including `v1` and `v2`.
    cfg.service(catalog_moved);
    cfg.service(library_moved);
    cfg.service(feed_moved);
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
        authenticated_config_for(libraries, None)
    }

    fn authenticated_config_for(
        libraries: &[(&str, &str)],
        authentication: Option<Authentication>,
    ) -> &'static Config {
        Box::leak(Box::new(config_of(libraries, authentication)))
    }

    fn config_of(libraries: &[(&str, &str)], authentication: Option<Authentication>) -> Config {
        Config {
            server: Server {
                ip: "127.0.0.1".to_string(),
                port: 8080,
                public_url: None,
                protocol: Protocol::Http,
            },
            authentication,
            calibre: Calibre {
                libraries: libraries
                    .iter()
                    .map(|(name, path)| {
                        (
                            name.to_string(),
                            Library { path: path.to_string(), author: None, landing_page: None },
                        )
                    })
                    .collect(),
            },
            catalog: Catalog::default(),
            dir: PathBuf::new(),
        }
    }

    fn landing_config(page: &str, dir: &Path) -> &'static Config {
        let mut config = config_of(&[("library", "tests/calibre")], None);
        config.dir = dir.to_path_buf();
        config.catalog.landing_page = Some(page.to_string());
        Box::leak(Box::new(config))
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

    // Guarded twice over: the check in `open_library`, and a connection that could not
    // create the file if the check were ever dropped.
    #[test]
    fn a_mistyped_path_does_not_create_a_database() {
        let dir = TempDir::new().unwrap();

        assert!(create_app(config_for(&[("library", dir.path().to_str().unwrap())])).is_err());
        assert!(!dir.path().join("metadata.db").exists());
    }

    #[test]
    fn a_library_is_opened_read_only() {
        let state = create_app(config_for(&[("library", "tests/calibre")])).unwrap();
        let db = state.db["library"].lock().unwrap();

        let err = db.execute("DELETE FROM books", []).unwrap_err().to_string();
        assert!(err.contains("readonly database"), "{}", err);
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
    fn a_library_named_after_a_route_refuses_to_start() {
        for name in RESERVED {
            let err = refusal(
                create_app(config_for(&[(name, "tests/calibre")])),
                "a library named after one of Orca's own routes must not start a server",
            );
            assert!(err.contains(&format!("library '{}': the name is reserved", name)), "{}", err);
        }
    }

    fn full() -> calibre::Counts {
        calibre::Counts { books: 7, authors: 4, tags: 7, series: 1, publishers: 2 }
    }

    #[test]
    fn a_library_naming_no_landing_page_is_served_the_built_in_one() {
        let state = create_app(config_for(&[("library", "tests/calibre")])).unwrap();

        assert!(state.pages.is_empty());

        let db = calibre::lock(&state.db["library"]);
        assert_eq!(state.landing("library").visible(&db, &full()).unwrap().len(), 6);
    }

    #[test]
    fn a_library_is_served_the_landing_page_its_config_names() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("page.toml"), "[[section]]\ntype = \"tags\"\ntitle = \"Genres\"\n")
            .unwrap();

        let state = create_app(landing_config("page.toml", dir.path())).unwrap();
        let db = calibre::lock(&state.db["library"]);
        let shown = state.landing("library").visible(&db, &full()).unwrap();

        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].title, "Genres");
    }

    #[test]
    fn a_landing_page_that_does_not_parse_refuses_to_start() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("page.toml"), "[[section]]\ntype = \"shelves\"\n").unwrap();

        let err = refusal(
            create_app(landing_config("page.toml", dir.path())),
            "a landing page that does not parse must not start a server",
        );

        assert!(err.contains("library 'library': landing page"), "{}", err);
        assert!(err.contains("unknown variant `shelves`"), "{}", err);
    }

    #[test]
    fn a_landing_page_that_is_not_there_refuses_to_start() {
        let dir = TempDir::new().unwrap();

        let err = refusal(
            create_app(landing_config("nowhere.toml", dir.path())),
            "a landing page that is not there must not start a server",
        );

        assert!(err.contains("nowhere.toml"), "{}", err);
    }

    fn authentication(logins: &[&str], public: &[&str]) -> Authentication {
        Authentication {
            login: logins.iter().map(|n| (n.to_string(), "hash".to_string())).collect(),
            public: public.iter().map(|p| public::Rule::new(p).unwrap()).collect(),
        }
    }

    // No credentials to check and nothing named public: every route would refuse every request
    #[test]
    fn an_empty_authentication_section_refuses_to_start() {
        let err = refusal(
            create_app(authenticated_config_for(
                &[("library", "tests/calibre")],
                Some(authentication(&[], &[])),
            )),
            "a server that can serve nobody must not start",
        );
        assert!(err.contains("no logins and names nothing public"), "{}", err);
    }

    #[test]
    fn a_public_entry_naming_no_configured_library_refuses_to_start() {
        let err = refusal(
            create_app(authenticated_config_for(
                &[("library", "tests/calibre")],
                Some(authentication(&["alice"], &["/libary"])),
            )),
            "a public entry naming an unknown library must not start a server",
        );
        assert!(err.contains("'libary', which is not a configured library"), "{}", err);
    }

    // It's ok to forgo authentication, if some libraries are listed as public
    #[test]
    fn public_libraries_without_logins_start() {
        assert!(create_app(authenticated_config_for(
            &[("library", "tests/calibre")],
            Some(authentication(&[], &["/library"])),
        ))
        .is_ok());
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
