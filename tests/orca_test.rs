use actix_web::{test, App, web, http::StatusCode, http::header};
use orca::{create_app, init, config};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use quick_xml::events::Event;
use quick_xml::reader::Reader;

#[actix_web::test]
async fn unauthorized_request() {

    let config = config::get().clone();
    let state = create_app(config);

    let app = test::init_service(
                App::new()
                    .app_data(web::Data::new(state))
                    .configure(init)
              ).await;

    let req = test::TestRequest::with_uri("/")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
async fn authorized_request() {
    let config = config::get().clone();
    let state = create_app(config);

    let app = test::init_service(
                App::new()
                    .app_data(web::Data::new(state))
                    .configure(init)
              ).await;

    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(is_opds(&content));
}

#[actix_web::test]
async fn list_books() {
    let config = config::get().clone();
    let state = create_app(config);

    let app = test::init_service(
                App::new()
                    .app_data(web::Data::new(state))
                    .configure(init)
              ).await;

    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/books")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert_eq!(count_books(&content), 3);
}

fn is_opds(content: &str) -> bool {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"feed" => break(true),
            Ok(Event::Eof) => panic!("Reached end of XML without finding feed"),
            Err(err) => panic!("Error reading XML: {:?}", err),
            _ => buf.clear(),
        }
    }
}

fn count_books(content: &str) -> usize {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut book_count = 0;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"entry" => book_count += 1,
            Ok(Event::Eof) => break,
            Err(err) => panic!("Error reading XML: {:?}", err),
            _ => buf.clear(),
        }
    }
    book_count
}

