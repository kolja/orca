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

// Atom requires <id> to be an IRI, so a bare Calibre row id won't cut it
#[test]
async fn book_ids_are_uuid_urns() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/books")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(content.contains("<id>urn:uuid:8b9f853c-7171-4f09-ba21-7304603a5128</id>"));
    assert!(!content.contains("<id>4</id>"));
}

// Atom requires a feed id to be permanent, so it is derived from the request
// path rather than the full URL
#[test]
async fn each_feed_carries_its_own_id() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (path, expected) in [
        ("/library", "<id>urn:orca:library</id>"),
        ("/library/books", "<id>urn:orca:library:books</id>"),
        ("/library/authors", "<id>urn:orca:library:authors</id>"),
        ("/library/tags", "<id>urn:orca:library:tags</id>"),
        ("/library/authors/5", "<id>urn:orca:library:authors:5</id>"),
        ("/library/tags/5", "<id>urn:orca:library:tags:5</id>"),
    ] {
        let req = test::TestRequest::with_uri(path)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

        assert!(content.contains(expected), "{} should contain {}", path, expected);
    }
}

#[test]
async fn entries_are_identified_by_what_they_refer_to() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (path, expected) in [
        ("/library", "<id>urn:orca:library:authors</id>"),
        ("/library", "<id>urn:orca:library:tags</id>"),
        ("/library", "<id>urn:orca:library:books</id>"),
        ("/library/authors", "<id>urn:orca:library:author:5</id>"),
        ("/library/tags", "<id>urn:orca:library:tag:5</id>"),
    ] {
        let req = test::TestRequest::with_uri(path)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

        assert!(content.contains(expected), "{} should contain {}", path, expected);
    }
}

// The id must survive a move behind a reverse proxy or a switch to https.
// The self link tracks the deployment; the id must not.
#[test]
async fn feed_ids_ignore_the_deployment() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/books")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .insert_header(("X-Forwarded-Proto", "https"))
        .insert_header(("X-Forwarded-Host", "books.example.com"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(content.contains("https://books.example.com/library/books"));
    assert!(content.contains("<id>urn:orca:library:books</id>"));
}

// Newest first, by the date the book entered the library.
#[test]
async fn recently_added_lists_the_newest_arrivals_first() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/new")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(content.contains("<title>library | Recently Added</title>"));
    assert_eq!(count_items(&content), 4);

    // Galileo was added 2026-07-28, Carroll's timestamp is back in 1865.
    let order: Vec<usize> = ["Galilei", "Kant", "Толстой", "Carroll"]
        .iter()
        .map(|name| content.find(name).unwrap_or_else(|| panic!("{} missing", name)))
        .collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(order, sorted, "books are not ordered newest first");
}

// A feed of books is an acquisition feed; only a feed of other feeds is a
// navigation feed. Clients use `kind` to decide which of the two to render.
#[test]
async fn feeds_of_books_are_advertised_as_acquisition_feeds() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (path, href) in [
        ("/library", "/library/books"),
        ("/library", "/library/new"),
        ("/library/authors", "/library/authors/5"),
        ("/library/tags", "/library/tags/5"),
    ] {
        let req = test::TestRequest::with_uri(path)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

        let link = content
            .split(&format!("href=\"{}\"", href))
            .nth(1)
            .unwrap_or_else(|| panic!("{} has no link to {}", path, href))
            .split("/>")
            .next()
            .unwrap_or_default()
            .to_string();
        assert!(
            link.contains("kind=acquisition"),
            "the link from {} to {} should be an acquisition feed, got:{}",
            path,
            href,
            link
        );
    }
}

// The http config has no [catalog] section at all.
#[test]
async fn an_unsigned_catalog_names_no_one() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for path in ["/library", "/library/books", "/library/authors", "/library/tags"] {
        let content = body_of(&app, path, &credentials).await;
        assert!(
            content.contains("<author><name>orca</name></author>"),
            "{} should fall back to the default author",
            path
        );
    }
}

// The https config sets `catalog.author`, and library2 overrides it.
#[test]
async fn a_library_may_name_its_own_author() {
    let app = setup(Https).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (path, expected) in [
        ("/library", "Jorge Luis Borges"),
        ("/library/books", "Jorge Luis Borges"),
        ("/library2", "Isaac Newton"),
        ("/library2/books", "Isaac Newton"),
        // The index spans libraries, so it can only use the catalog default.
        ("/", "Jorge Luis Borges"),
    ] {
        let content = body_of(&app, path, &credentials).await;
        assert!(
            content.contains(&format!("<author><name>{}</name></author>", expected)),
            "{} should be signed by {}",
            path,
            expected
        );
    }
}

// <author> is whoever publishes the catalog; the software belongs in <generator>.
#[test]
async fn feeds_name_the_software_that_built_them() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/library", &credentials).await;

    let generator = content
        .split("<generator")
        .nth(1)
        .expect("the feed should carry a <generator>")
        .split("</generator>")
        .next()
        .unwrap_or_default();

    assert!(generator.ends_with(">orca"), "got:{}", generator);
    assert!(
        generator.contains(&format!(r#"version="{}""#, env!("CARGO_PKG_VERSION"))),
        "the generator should carry the running version, got:{}",
        generator
    );
}

// RFC 4287 §3.3: 'T' must separate date and time
#[test]
async fn dates_are_rfc3339() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/library/books")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    // Alice in Wonderland: pubdate 2008-06-27, last_modified 2024-11-30.
    assert!(content.contains("<published>2008-06-27T00:00:00+00:00</published>"));
    assert!(content.contains("<updated>2024-11-30T10:26:17.544488+00:00</updated>"));

    // No date anywhere in the feed may keep Calibre's space separator.
    for tag in ["published", "updated"] {
        for tail in content.split(&format!("<{}>", tag)).skip(1) {
            let value = tail.split('<').next().unwrap_or_default();
            assert!(!value.contains(' '), "<{}>{}</{}> is not RFC 3339", tag, value, tag);
        }
    }
}

// The feed is as fresh as the most recently touched book in the library.
#[test]
async fn feeds_report_the_latest_change_in_the_library() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    // Galileo, id 6, is the most recently modified book: 2026-07-28.
    for path in ["/library", "/library/books", "/library/authors", "/library/tags"] {
        let req = test::TestRequest::with_uri(path)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

        assert!(
            content.contains("<updated>2026-07-28T19:57:00.000000+00:00</updated>"),
            "{} should report the library's latest change",
            path
        );
    }
}

// Atom requires an <updated> on every entry, not just on the feed.
#[test]
async fn every_entry_carries_an_updated() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for path in ["/library", "/library/books", "/library/authors", "/library/tags"] {
        let req = test::TestRequest::with_uri(path)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

        assert_eq!(
            content.matches("<updated>").count(),
            count_items(&content) + 1,
            "{} should carry an <updated> on the feed and on every entry",
            path
        );
    }
}

/// What a handler hands `books.xml.tera`, for the tests below that render the
/// template directly rather than through the server.
fn book_feed_context(config: &Config, books: serde_json::Value) -> tera::Context {
    let mut context = tera::Context::new();
    context.insert("config", config);
    context.insert("lib", "library");
    context.insert("base", "http://localhost:8888");
    context.insert("self_url", "http://localhost:8888/library/books");
    context.insert("feed_id", "urn:orca:library:books");
    context.insert("updated", "2026-01-01T00:00:00+00:00");
    context.insert("feed_title", "library | 1 books");
    context.insert("author", "orca");
    context.insert("version", "0.0.0");
    context.insert("repository", "https://example.invalid/orca");
    context.insert("books", &books);
    context
}

#[test]
async fn a_book_without_a_uuid_falls_back_to_a_library_scoped_urn() {
    let state = create_app(&TEST_HTTP_CONFIG).expect("Failed to create app");
    let context = book_feed_context(state.config, serde_json::json!([{
        "id": 999,
        "uuid": "",
        "title": "A Book From The Before Times",
        "pubdate": "2026-01-01T00:00:00+00:00",
        "updated": "2026-01-01T00:00:00+00:00",
        "synopsis": "",
        "authors": [],
        "formats": ["epub"]
    }]));

    let content = state
        .templates
        .render("books.xml.tera", &context)
        .expect("Failed to render books template");

    assert!(content.contains("<id>urn:orca:library:book:999</id>"));
}

#[test]
async fn book_metadata_is_xml_escaped() {
    let state = create_app(&TEST_HTTP_CONFIG).expect("Failed to create app");
    let context = book_feed_context(state.config, serde_json::json!([{
        "id": 999,
        "title": "Fish & Chips <Special>",
        "pubdate": "2026-01-01T00:00:00+00:00",
        "updated": "2026-01-01T00:00:00+00:00",
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
    let context = book_feed_context(state.config, serde_json::json!([{
        "id": 999,
        "title": "Fish & Chips",
        "pubdate": "2026-01-01T00:00:00+00:00",
        "updated": "2026-01-01T00:00:00+00:00",
        "synopsis": "Science fact & science fiction",
        "authors": [{"id": 1, "name": "O'Brien & Sons"}],
        "formats": ["epub", "pdf", "mobi", "cbz", "lrf"]
    }]));

    let content = state
        .templates
        .render("books.xml.tera", &context)
        .expect("Failed to render books template");

    assert!(content.contains(r#"type="application/epub+zip""#));
    assert!(content.contains(r#"type="application/pdf""#));
    assert!(content.contains(r#"type="application/x-mobipocket-ebook""#));
    assert!(content.contains(r#"type="application/vnd.comicbook+zip""#));
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

// Alice is stored as epub and azw3. A format Orca has no mime type of its own
// for must still reach the client: dropping it can leave a book with nothing to
// acquire at all.
#[test]
async fn formats_beyond_the_usual_three_are_offered_too() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/library/books", &credentials).await;
    assert_eq!(count_links(&content, "/library/file/4/epub"), 1);
    assert_eq!(count_links(&content, "/library/file/4/azw3"), 1);
    assert!(content.contains(r#"type="application/vnd.amazon.mobi8-ebook""#));

    let req = test::TestRequest::with_uri("/library/file/4/azw3")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "the offered format must download");

    // AZW3 is Amazon's KF8, which still carries a PalmDB header.
    let body = test::read_body(resp).await;
    assert_eq!(&body[60..68], b"BOOKMOBI");
}

// Book 5 is an epub and nothing else, book 4 has no mobi. Asking for a format
// the library does not hold is a 404 -- it used to be a 500, because the path
// was built before anyone checked whether that file existed.
#[test]
async fn a_format_the_library_does_not_hold_is_404() {
    let app = setup(Http).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for uri in ["/library/file/5/pdf", "/library/file/4/mobi"] {
        let req = test::TestRequest::with_uri(uri)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{} should be 404", uri);
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
