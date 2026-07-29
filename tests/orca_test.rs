use actix_web::{test, App, web};
use actix_web::http::{header, StatusCode};
use actix_web::dev::{Service, ServiceResponse};
use actix_http::Request;
use orca::{create_app, init};
use orca::config::{Config, read_config};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use quick_xml::{events::Event, reader::Reader};
use once_cell::sync::Lazy;

enum Protocol {
    Http,
    Https,
}
use Protocol::{Http, Https};

static TEST_HTTP_CONFIG: Lazy<Config> = Lazy::new(|| {
    read_config("tests/orca.http.test.toml").expect("Failed to read test config")
});

static TEST_HTTPS_CONFIG: Lazy<Config> = Lazy::new(|| {
    read_config("tests/orca.https.test.toml").expect("Failed to read test config")
});

async fn setup(protocol: Protocol) -> impl Service<Request, Response = ServiceResponse, Error = actix_web::Error> {
    let state = match protocol {
        Http => create_app(&TEST_HTTP_CONFIG),
        Https => create_app(&TEST_HTTPS_CONFIG),
    }.expect("Failed to create app");
    test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(init)
          ).await
}

// ------- Http Tests -------

#[test]
async fn health() {
    let app = setup(Http).await;
    let req = test::TestRequest::with_uri("/health").to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
}

#[test]
async fn unauthorized_request() {
    let app = setup(Http).await;
    let req = test::TestRequest::with_uri("/library")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[test]
async fn authorized_request() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(is_opds(&content));
}

#[test]
async fn list_books() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/books")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert_eq!(count_items(&content), 4);
}

#[test]
async fn book_metadata_is_xml_escaped() {
    let state = create_app(&TEST_HTTP_CONFIG).expect("Failed to create app");
    let mut context = tera::Context::new();
    context.insert("config", &state.config);
    context.insert("lib", "library");
    context.insert("base", "http://localhost:8888");
    context.insert("self_url", "http://localhost:8888/library/books");
    context.insert("books", &serde_json::json!([{
        "id": 999,
        "title": "Fish & Chips <Special>",
        "pubdate": "2026-01-01T00:00:00+00:00",
        "synopsis": "Science fact & science fiction",
        "authors": [{"id": 1, "name": "A & B"}],
        "formats": ["epub"]
    }]));

    let content = state
        .templates
        .render("books.xml.tera", &context)
        .expect("Failed to render books template");

    assert!(content.contains("Fish &amp; Chips &lt;Special&gt;"));
    assert!(content.contains("Science fact &amp; science fiction"));
    assert!(content.contains("A &amp; B"));
    assert_eq!(count_items(&content), 1);
}

// Auto-escaping would otherwise encode the `/` in mime types as `&#x2F;`.
#[test]
async fn mime_types_are_not_escaped() {
    let state = create_app(&TEST_HTTP_CONFIG).expect("Failed to create app");
    let mut context = tera::Context::new();
    context.insert("config", &state.config);
    context.insert("lib", "library");
    context.insert("base", "http://localhost:8888");
    context.insert("self_url", "http://localhost:8888/library/books");
    context.insert("books", &serde_json::json!([{
        "id": 999,
        "title": "Fish & Chips",
        "pubdate": "2026-01-01T00:00:00+00:00",
        "synopsis": "Science fact & science fiction",
        "authors": [{"id": 1, "name": "O'Brien & Sons"}],
        "formats": ["epub", "pdf", "mobi", "cbz"]
    }]));

    let content = state
        .templates
        .render("books.xml.tera", &context)
        .expect("Failed to render books template");

    assert!(content.contains(r#"type="application/epub+zip""#));
    assert!(content.contains(r#"type="application/pdf""#));
    assert!(content.contains(r#"type="application/x-mobipocket-ebook""#));
    // Unrecognised formats fall back to a generic mime type.
    assert!(content.contains(r#"type="application/octet-stream""#));

    // Escaping of the actual metadata still happens.
    assert!(content.contains("Fish &amp; Chips"));
    assert_eq!(count_items(&content), 1);
}

#[test]
async fn list_authors() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/authors")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert_eq!(count_items(&content), 5);
}

#[test]
async fn list_books_by_immanuel_kant() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/authors/5")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert_eq!(count_items(&content), 1);
}

#[test]
async fn list_tags() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/tags")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert_eq!(count_items(&content), 5);
}

#[test]
async fn list_books_tagged_fiction() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/tags/5")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert_eq!(count_items(&content), 2);
}

#[test]
async fn download_cover() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/cover/5")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    assert_eq!(resp.headers().get("content-type").unwrap(), "image/jpeg");
}

#[test]
async fn download_epub() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/file/5/epub")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    assert_eq!(resp.headers().get("content-type").unwrap(), "application/epub+zip");
}

#[test]
async fn download_mobi() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/file/6/mobi")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    assert_eq!(&body[60..68], b"BOOKMOBI");
}

// A book id that is not in the database must be a 404.
#[test]
async fn missing_ids_return_404_and_leave_the_library_usable() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let auth = || (header::AUTHORIZATION, format!("Basic {}", credentials));

    for uri in ["/library/cover/99999", "/library/file/99999/epub"] {
        let req = test::TestRequest::with_uri(uri).insert_header(auth()).to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{} should be 404", uri);
    }

    let req = test::TestRequest::with_uri("/library/books").insert_header(auth()).to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "library unusable after a missing-id request");
}

// Tags and authors that match nothing are an empty feed, not an error.
#[test]
async fn unmatched_queries_render_an_empty_feed() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for uri in ["/library/tags/99999", "/library/authors/99999"] {
        let req = test::TestRequest::with_uri(uri)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success(), "{} should succeed", uri);

        let body = test::read_body(resp).await;
        let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");
        assert!(is_opds(&content));
        assert_eq!(count_items(&content), 0, "{} should have no entries", uri);
    }
}

// A library that is not configured is a 404
#[test]
async fn unknown_library_is_not_found() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for uri in ["/nosuchlib", "/nosuchlib/books", "/nosuchlib/tags",
                "/nosuchlib/authors", "/nosuchlib/tags/5", "/nosuchlib/authors/5",
                "/nosuchlib/cover/5", "/nosuchlib/file/5/epub"] {
        let req = test::TestRequest::with_uri(uri)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{} should be 404", uri);
    }
}

// One acquisition link per format
#[test]
async fn each_format_is_offered_exactly_once() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    // Book 6 has two authors and two formats; book 4 has three tags.
    for uri in ["/library/books", "/library/authors/6", "/library/authors/7"] {
        let content = body_of(&app, uri, &credentials).await;
        assert_eq!(count_links(&content, "/library/file/6/epub"), 1, "{}", uri);
        assert_eq!(count_links(&content, "/library/file/6/mobi"), 1, "{}", uri);
    }

    for uri in ["/library/books", "/library/authors/4", "/library/tags/5"] {
        let content = body_of(&app, uri, &credentials).await;
        assert_eq!(count_links(&content, "/library/file/4/epub"), 1, "{}", uri);
    }
}

#[test]
async fn untagged_books_still_appear_in_author_feeds() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/library/authors/6", &credentials).await;
    assert_eq!(count_items(&content), 1);
    assert!(content.contains("The sidereal messenger of Galileo Galilei"));
}

// A co-authored book is one entry that credits everyone, not one entry per
// author and not an arbitrary single author.
#[test]
async fn co_authored_books_list_every_author() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/library/books", &credentials).await;
    assert_eq!(count_items(&content), 4, "the co-authored book must not be duplicated");
    assert!(content.contains("<name>Galileo Galilei</name>"));
    assert!(content.contains("<name>Johannes Kepler</name>"));
    // Author links must point at a route that exists.
    assert!(content.contains("<uri>/library/authors/6</uri>"));
    assert!(!content.contains("<uri>/author/"));
}

// The self link must name the host the client actually reached, never the bind
// address: clients like Moon+ Reader resolve entry links against it, and a
// wildcard bind (0.0.0.0) is not connectable from anywhere.
#[test]
async fn self_link_follows_forwarded_headers() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/tags")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .insert_header(("X-Forwarded-Proto", "https"))
        .insert_header(("X-Forwarded-Host", "orca.example.com"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(content.contains(r#"rel="self" href="https://orca.example.com/library/tags""#));
    assert!(content.contains(r#"rel="start" href="https://orca.example.com/""#));
    assert!(!content.contains("0.0.0.0"));
}

// Without a proxy the Host header is the only thing the client can reach us by.
#[test]
async fn self_link_falls_back_to_host_header() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .insert_header((header::HOST, "books.local:8888"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(content.contains(r#"rel="self" href="http://books.local:8888/library""#));
}

// With a single library there is nothing to choose from, so the root sends the
// client straight to it instead of showing a one-entry catalog.
#[test]
async fn single_library_redirects_to_it() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/library");
}

// ------- Https Tests -------

// The https config registers tests/calibre twice, so the root is a real
// catalog listing every library rather than a redirect.
#[test]
async fn multiple_libraries_are_listed() {
    let app = setup(Https).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/", &credentials).await;

    assert!(is_opds(&content));
    assert_eq!(count_items(&content), 2);
    assert!(content.contains("<title>library</title>"));
    assert!(content.contains("<title>library2</title>"));
    assert!(content.contains(r#"href="/library2""#));
}

#[test]
async fn unauthorized_request_https() {
    let app = setup(Https).await;
    let req = test::TestRequest::with_uri("/library")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[test]
async fn unauthorized_request_public_route_https() {
    let app = setup(Https).await;
    let req = test::TestRequest::with_uri("/health")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
}

#[test]
async fn authorized_request_https() {
    let app = setup(Https).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(is_opds(&content));
}

#[test]
async fn authorized_request_to_public_route_https() {
    let app = setup(Https).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/health")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
}

// ------- Helper Functions -------

fn is_opds(content: &str) -> bool {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"feed" => break true,
            Ok(Event::Eof) => panic!("Reached end of XML without finding feed"),
            Err(err) => panic!("Error reading XML: {:?}", err),
            _ => buf.clear(),
        }
    }
}

async fn body_of(
    app: &impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    uri: &str,
    credentials: &str,
) -> String {
    let req = test::TestRequest::with_uri(uri)
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(app, req).await;
    assert!(resp.status().is_success(), "{} should succeed", uri);
    let body = test::read_body(resp).await;
    String::from_utf8(body.to_vec()).expect("Failed to convert to String")
}

fn count_links(content: &str, href: &str) -> usize {
    content.matches(&format!(r#"href="{}""#, href)).count()
}

fn count_items(content: &str) -> usize {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut item_count = 0;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"entry" => item_count += 1,
            Ok(Event::Eof) => break,
            Err(err) => panic!("Error reading XML: {:?}", err),
            _ => buf.clear(),
        }
    }
    item_count
}
