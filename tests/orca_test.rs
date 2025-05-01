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
    };
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

    assert_eq!(count_items(&content), 3);
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

    assert_eq!(count_items(&content), 3);
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

// ------- Https Tests -------

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
