use actix_web::test;
use actix_web::http::{header, StatusCode};
use actix_web::dev::{Service, ServiceResponse};
use actix_http::Request;
use orca::create_app;
use orca::config::{Config, read_config};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use quick_xml::{events::Event, reader::Reader};
use once_cell::sync::Lazy;

/// the feeds carry no relative links
const BASE: &str = "http://localhost:8080";

static ONE_LIBRARY: Lazy<Config> = Lazy::new(|| {
    read_config("tests/orca.http.test.toml").expect("Failed to read test config")
});

/// The second one overrides `catalog.author`
static TWO_LIBRARIES: Lazy<Config> = Lazy::new(|| {
    read_config("tests/orca.https.test.toml").expect("Failed to read test config")
});

/// Four libraries: one public, one public as JSON only, and two authenticated ones
static PARTLY_PUBLIC: Lazy<Config> = Lazy::new(|| {
    read_config("tests/orca.public.test.toml").expect("Failed to read test config")
});

/// One library, served with a landing page of named shelves and no login
static SHELVES: Lazy<Config> = Lazy::new(|| {
    read_config("tests/orca.shelves.test.toml").expect("Failed to read test config")
});

static INLINE: Lazy<Config> = Lazy::new(|| {
    read_config("tests/orca.inline.test.toml").expect("Failed to read test config")
});

async fn setup(config: &'static Config) -> impl Service<Request, Response = ServiceResponse, Error = actix_web::Error> {
    let state = create_app(config).expect("Failed to create app");
    test::init_service(orca::app(state)).await
}

// ------- One library, all of it behind a login -------

#[test]
async fn health() {
    let app = setup(&ONE_LIBRARY).await;
    let req = test::TestRequest::with_uri("/health").to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
}

#[test]
async fn unauthorized_request() {
    let app = setup(&ONE_LIBRARY).await;
    let req = test::TestRequest::with_uri("/v1/library")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[test]
async fn authorized_request() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library")
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/books")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert_eq!(count_items(&content), 7);
}

// Atom requires <id> to be an IRI, so a bare Calibre row id won't cut it
#[test]
async fn book_ids_are_uuid_urns() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/books")
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (path, expected) in [
        ("/v1/library", "<id>urn:orca:library</id>"),
        ("/v1/library/books", "<id>urn:orca:library:books</id>"),
        ("/v1/library/authors", "<id>urn:orca:library:authors</id>"),
        ("/v1/library/tags", "<id>urn:orca:library:tags</id>"),
        ("/v1/library/series", "<id>urn:orca:library:series</id>"),
        ("/v1/library/publishers", "<id>urn:orca:library:publishers</id>"),
        ("/v1/library/authors/5", "<id>urn:orca:library:authors:5</id>"),
        ("/v1/library/tags/5", "<id>urn:orca:library:tags:5</id>"),
        ("/v1/library/series/1", "<id>urn:orca:library:series:1</id>"),
        ("/v1/library/publishers/4", "<id>urn:orca:library:publishers:4</id>"),
        ("/v1/library/search?query=alice", "<id>urn:orca:library:search</id>"),
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (path, expected) in [
        ("/v1/library", "<id>urn:orca:library:authors</id>"),
        ("/v1/library", "<id>urn:orca:library:tags</id>"),
        ("/v1/library", "<id>urn:orca:library:books</id>"),
        ("/v1/library", "<id>urn:orca:library:series</id>"),
        ("/v1/library", "<id>urn:orca:library:publishers</id>"),
        ("/v1/library/authors", "<id>urn:orca:library:author:5</id>"),
        ("/v1/library/tags", "<id>urn:orca:library:tag:5</id>"),
        ("/v1/library/publishers", "<id>urn:orca:library:publisher:4</id>"),
        // The one category whose singular is its plural.
        ("/v1/library/series", "<id>urn:orca:library:series:1</id>"),
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/books")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .insert_header(("X-Forwarded-Proto", "https"))
        .insert_header(("X-Forwarded-Host", "books.example.com"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(content.contains("https://books.example.com/v1/library/books"));
    assert!(content.contains("<id>urn:orca:library:books</id>"));
}

// Newest first, by the date the book entered the library.
#[test]
async fn recently_added_lists_the_newest_arrivals_first() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/new")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(content.contains("<title>library | Recently Added</title>"));
    assert_eq!(count_items(&content), 7);

    // The three Astounding Stories arrived in August 2026, Galileo in July;
    // Carroll's timestamp is back in 1865.
    let order: Vec<usize> = ["Smith", "Lovecraft", "Wright", "Galilei", "Kant", "Толстой", "Carroll"]
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (path, href) in [
        ("/v1/library", "/v1/library/books"),
        ("/v1/library", "/v1/library/new"),
        ("/v1/library/authors", "/v1/library/authors/5"),
        ("/v1/library/tags", "/v1/library/tags/5"),
        ("/v1/library/series", "/v1/library/series/1"),
        ("/v1/library/publishers", "/v1/library/publishers/4"),
    ] {
        let content = body_of(&app, path, &credentials).await;

        let link = link_to(&content, href);
        assert!(
            link.contains("kind=acquisition"),
            "the link from {} to {} should be an acquisition feed, got:{}",
            path,
            href,
            link
        );
    }
}

// This library's config has no [catalog] section at all.
#[test]
async fn an_unsigned_catalog_names_no_one() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for path in ["/v1/library", "/v1/library/books", "/v1/library/authors", "/v1/library/tags"] {
        let content = body_of(&app, path, &credentials).await;
        assert!(
            content.contains("<author><name>orca</name></author>"),
            "{} should fall back to the default author",
            path
        );
    }
}

// `catalog.author` is set for both, and library2 overrides it.
#[test]
async fn a_library_may_name_its_own_author() {
    let app = setup(&TWO_LIBRARIES).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (path, expected) in [
        ("/v1/library", "Jorge Luis Borges"),
        ("/v1/library/books", "Jorge Luis Borges"),
        ("/v1/library2", "Isaac Newton"),
        ("/v1/library2/books", "Isaac Newton"),
        // The catalog spans libraries, so it can only use the catalog default.
        ("/v1", "Jorge Luis Borges"),
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/v1/library", &credentials).await;

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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/books")
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    // Galactic Patrol, id 9, is the most recently modified book: 2026-08-03.
    for path in ["/v1/library", "/v1/library/books", "/v1/library/authors", "/v1/library/tags"] {
        let req = test::TestRequest::with_uri(path)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

        assert!(
            content.contains("<updated>2026-08-03T20:04:55.000000+00:00</updated>"),
            "{} should report the library's latest change",
            path
        );
    }
}

// Atom requires an <updated> on every entry, not just on the feed.
#[test]
async fn every_entry_carries_an_updated() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for path in ["/v1/library", "/v1/library/books", "/v1/library/authors", "/v1/library/tags"] {
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
    context.insert("catalog", "http://localhost:8888/v1");
    context.insert("self_url", "http://localhost:8888/v1/library/books");
    context.insert("feed_id", "urn:orca:library:books");
    context.insert("updated", "2026-01-01T00:00:00+00:00");
    context.insert("feed_title", "library | All Books");
    context.insert("author", "orca");
    context.insert("version", "0.0.0");
    context.insert("repository", "https://example.invalid/orca");
    context.insert("kind", "acquisition");
    context.insert("up", &true);
    context.insert("paging", &serde_json::json!({
        "total": 1, "per_page": 50, "start": 1, "links": [],
    }));
    context.insert("books", &books);
    context
}

fn a_book(title: &str, formats: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": 999,
        "uuid": "",
        "title": title,
        "pubdate": "2026-01-01T00:00:00+00:00",
        "updated": "2026-01-01T00:00:00+00:00",
        "synopsis": "",
        "has_cover": true,
        "formats": formats,
        "authors": [],
        "languages": [],
        "tags": [],
        "publisher": null,
        "series": null,
    })
}

#[test]
async fn a_book_without_a_uuid_falls_back_to_a_library_scoped_urn() {
    let state = create_app(&ONE_LIBRARY).expect("Failed to create app");
    let context = book_feed_context(
        state.config,
        serde_json::json!([a_book("A Book From The Before Times", serde_json::json!([{"name": "epub", "size": 1}]))]),
    );

    let content = state
        .templates
        .render("books.xml.tera", &context)
        .expect("Failed to render books template");

    assert!(content.contains("<id>urn:orca:library:book:999</id>"));
}

#[test]
async fn book_metadata_is_xml_escaped() {
    let state = create_app(&ONE_LIBRARY).expect("Failed to create app");
    let mut book = a_book("Fish & Chips <Special>", serde_json::json!([{"name": "epub", "size": 1}]));
    book["synopsis"] = serde_json::json!("Science fact & science fiction");
    book["authors"] = serde_json::json!([{"id": 1, "name": "A & B"}]);
    book["tags"] = serde_json::json!([{"id": 1, "name": "Punch & Judy"}]);
    book["publisher"] = serde_json::json!({"id": 1, "name": "Faber & Faber"});
    book["series"] = serde_json::json!({"id": 1, "name": "Cloak & Dagger", "index": 1.0});
    let context = book_feed_context(state.config, serde_json::json!([book]));

    let content = state
        .templates
        .render("books.xml.tera", &context)
        .expect("Failed to render books template");

    assert!(content.contains("Fish &amp; Chips &lt;Special&gt;"));
    assert!(content.contains("Science fact &amp; science fiction"));
    assert!(content.contains("A &amp; B"));
    // Everything the newer entry elements carry needs escaping too.
    assert!(content.contains(r#"term="Punch &amp; Judy""#));
    assert!(content.contains("<dcterms:publisher>Faber &amp; Faber</dcterms:publisher>"));
    assert!(content.contains(r#"title="Cloak &amp; Dagger""#));
    assert_eq!(count_items(&content), 1);
}

// Auto-escaping would otherwise encode the `/` in mime types as `&#x2F;`.
#[test]
async fn mime_types_are_not_escaped() {
    let state = create_app(&ONE_LIBRARY).expect("Failed to create app");
    let mut book = a_book(
        "Fish & Chips",
        serde_json::json!([
            {"name": "epub", "size": 1}, {"name": "pdf", "size": 2}, {"name": "mobi", "size": 3},
            {"name": "cbz", "size": 4}, {"name": "foo", "size": 5},
        ]),
    );
    book["synopsis"] = serde_json::json!("Science fact & science fiction");
    book["authors"] = serde_json::json!([{"id": 1, "name": "O'Brien & Sons"}]);
    let context = book_feed_context(state.config, serde_json::json!([book]));

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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/authors")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert_eq!(count_items(&content), 8);
}

#[test]
async fn list_books_by_immanuel_kant() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/authors/5")
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/tags")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert_eq!(count_items(&content), 7);
}

#[test]
async fn list_books_tagged_fiction() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/tags/5")
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
    let app = setup(&ONE_LIBRARY).await;
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
    let app = setup(&ONE_LIBRARY).await;
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
    let app = setup(&ONE_LIBRARY).await;
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let auth = || (header::AUTHORIZATION, format!("Basic {}", credentials));

    for uri in ["/library/cover/99999", "/library/file/99999/epub"] {
        let req = test::TestRequest::with_uri(uri).insert_header(auth()).to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{} should be 404", uri);
    }

    let req = test::TestRequest::with_uri("/v1/library/books").insert_header(auth()).to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success(), "library unusable after a missing-id request");
}

// An id that names no author and no tag is a 404, the same as under `/v2`.
#[test]
async fn a_shelf_that_is_not_there_is_404() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for uri in ["/v1/library/tags/99999", "/v1/library/authors/99999",
                "/v1/library/series/99999", "/v1/library/publishers/99999"] {
        let req = test::TestRequest::with_uri(uri)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{} should be 404", uri);
    }
}

// A library that is not configured is a 404
#[test]
async fn unknown_library_is_not_found() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for uri in ["/v1/nosuchlib", "/v1/nosuchlib/books", "/v1/nosuchlib/tags",
                "/v1/nosuchlib/authors", "/v1/nosuchlib/tags/5", "/v1/nosuchlib/authors/5",
                "/v1/nosuchlib/series", "/v1/nosuchlib/publishers", "/v1/nosuchlib/series/1",
                "/v1/nosuchlib/publishers/4", "/v1/nosuchlib/search?query=alice",
                "/v1/nosuchlib/opensearch.xml",
                "/nosuchlib/cover/5", "/nosuchlib/file/5/epub"] {
        let req = test::TestRequest::with_uri(uri)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{} should be 404", uri);
    }
}

// trailing slashes and double slashes are normalized
#[test]
async fn a_sloppy_url_is_the_url_it_meant() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (sloppy, meant) in [
        ("/v1/library/", "/v1/library"),
        ("/v1/library/books/", "/v1/library/books"),
        ("/v1/library/authors/", "/v1/library/authors"),
        ("/v1/library/search/?query=alice", "/v1/library/search?query=alice"),
        ("//v1//library", "/v1/library"),
        ("/v1/library//books", "/v1/library/books"),
        ("/v2/library/", "/v2/library"),
    ] {
        assert_eq!(
            body_of(&app, sloppy, &credentials).await,
            body_of(&app, meant, &credentials).await,
            "{} should answer as {} does",
            sloppy,
            meant
        );
    }

    for (sloppy, meant) in [("/v1/", "/v1"), ("/v2/", "/v2")] {
        assert_eq!(
            answer_to(&app, sloppy, &credentials).await,
            answer_to(&app, meant, &credentials).await,
            "{} should answer as {} does",
            sloppy,
            meant
        );
    }
}

#[test]
async fn a_public_library_is_public_in_both_feeds() {
    let app = setup(&PARTLY_PUBLIC).await;

    for path in ["/v1/library", "/v1/library/books", "/v1/library/authors/5",
                 "/v1/library/search?query=alice", "/v2/library", "/v2/library/books",
                 "/v2/library/book/4", "/library/cover/5", "/library/file/4/epub"] {
        let req = test::TestRequest::with_uri(path).to_request();
        let resp = test::call_service(&app, req).await;

        assert!(resp.status().is_success(), "{} should need no credentials", path);
    }

    // the redirect must point to a public resource (rather than to a 401)
    for path in ["/library", "/library/books"] {
        let req = test::TestRequest::with_uri(path).to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::FOUND, "{} should redirect", path);

        let target = resp.headers().get(header::LOCATION).unwrap().to_str().unwrap();
        let req = test::TestRequest::with_uri(target.trim_start_matches(BASE)).to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success(), "{} leads to a 401", path);
    }
}

// A pattern naming a version opens that version's feeds plus the covers and downloads
#[test]
async fn a_feed_opens_the_files_it_points_at() {
    let app = setup(&PARTLY_PUBLIC).await;

    for path in ["/v2/json", "/v2/json/books", "/v2/json/book/4",
                 "/json/cover/5", "/json/file/4/epub"] {
        let req = test::TestRequest::with_uri(path).to_request();
        let resp = test::call_service(&app, req).await;

        assert!(resp.status().is_success(), "{} should need no credentials", path);
    }

    // The version it does not name still requires authentication
    for path in ["/v1/json", "/v1/json/books"] {
        let req = test::TestRequest::with_uri(path).to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{} should ask for a login", path);
    }
}

// Public is per entry, not per server
#[test]
async fn a_library_no_entry_names_stays_private() {
    let app = setup(&PARTLY_PUBLIC).await;

    for path in ["/v1/private", "/v1/private/books", "/v2/private", "/private/books",
                 "/private/cover/5", "/private/file/4/epub"] {
        let req = test::TestRequest::with_uri(path).to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{} should ask for a login", path);
    }
}

#[test]
async fn a_library_that_starts_like_a_public_one_is_still_private() {
    let app = setup(&PARTLY_PUBLIC).await;

    for path in ["/v1/library2", "/v1/library2/books", "/v2/library2", "/v2/library2/books",
                 "/library2/books", "/library2/cover/5", "/library2/file/4/epub"] {
        let req = test::TestRequest::with_uri(path).to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{} should ask for a login", path);
    }
}

// A catalog lists what the client can open
#[test]
async fn a_catalog_lists_only_what_the_caller_can_reach() {
    let app = setup(&PARTLY_PUBLIC).await;

    // `json` is public as JSON only, so to a guest the Atom catalog holds one
    // library -- and a catalog with only one entry redirects
    let req = test::TestRequest::with_uri("/v1").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    assert_eq!(
        resp.headers().get(header::LOCATION).unwrap().to_str().unwrap(),
        format!("{}/v1/library", BASE)
    );

    // Two under JSON, and neither of the private ones show up
    let body = body_of(&app, "/v2", "").await;
    assert!(body.contains(r#""title":"library""#), "{}", body);
    assert!(body.contains(r#""title":"json""#), "{}", body);
    assert!(!body.contains("library2"), "{}", body);
    assert!(!body.contains("private"), "{}", body);

    // Alice sees all four, in both formats.
    let credentials = BASE64.encode("alice:secretpassword");
    for uri in ["/v1", "/v2"] {
        let body = body_of(&app, uri, &credentials).await;
        for lib in ["library", "library2", "json", "private"] {
            assert!(body.contains(lib), "{} should list {}\n{}", uri, lib, body);
        }
    }
}

// One acquisition link per format
#[test]
async fn each_format_is_offered_exactly_once() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    // Book 6 has two authors and two formats; book 4 has three tags.
    for uri in ["/v1/library/books", "/v1/library/authors/6", "/v1/library/authors/7"] {
        let content = body_of(&app, uri, &credentials).await;
        assert_eq!(count_links(&content, "/library/file/6/epub"), 1, "{}", uri);
        assert_eq!(count_links(&content, "/library/file/6/mobi"), 1, "{}", uri);
    }

    for uri in ["/v1/library/books", "/v1/library/authors/4", "/v1/library/tags/5"] {
        let content = body_of(&app, uri, &credentials).await;
        assert_eq!(count_links(&content, "/library/file/4/epub"), 1, "{}", uri);
    }
}

// Alice is stored as epub and azw3. A format Orca has no mime type of its own
// for must still reach the client: dropping it can leave a book with nothing to
// acquire at all.
#[test]
async fn formats_beyond_the_usual_three_are_offered_too() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/v1/library/books", &credentials).await;
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
    let app = setup(&ONE_LIBRARY).await;
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
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/v1/library/authors/6", &credentials).await;
    assert_eq!(count_items(&content), 1);
    assert!(content.contains("The sidereal messenger of Galileo Galilei"));
}

// A co-authored book is one entry that credits everyone, not one entry per
// author and not an arbitrary single author.
#[test]
async fn co_authored_books_list_every_author() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/v1/library/books", &credentials).await;
    assert_eq!(count_items(&content), 7, "the co-authored book must not be duplicated");
    assert!(content.contains("<name>Galileo Galilei</name>"));
    assert!(content.contains("<name>Johannes Kepler</name>"));
    // Author links must point at a route that exists.
    assert!(content.contains(&format!("<uri>{}/v1/library/authors/6</uri>", BASE)));
    assert!(!content.contains("<uri>/author/"));
}

// The self link must name the host the client actually reached, never the bind
// address: clients like Moon+ Reader resolve entry links against it, and a
// wildcard bind (0.0.0.0) is not connectable from anywhere.
#[test]
async fn self_link_follows_forwarded_headers() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library/tags")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .insert_header(("X-Forwarded-Proto", "https"))
        .insert_header(("X-Forwarded-Host", "orca.example.com"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(content.contains(r#"rel="self" href="https://orca.example.com/v1/library/tags""#));
    assert!(content.contains(r#"rel="start" href="https://orca.example.com/v1""#));
    assert!(!content.contains("0.0.0.0"));
}

// Without a proxy the Host header is the only thing the client can reach us by.
#[test]
async fn self_link_falls_back_to_host_header() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .insert_header((header::HOST, "books.local:8888"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");

    assert!(content.contains(r#"rel="self" href="http://books.local:8888/v1/library""#));
}

// Two hops. First `/` -> `/v1`
// But there is only one library to pick, so `/v1` -> `/v1/library`
#[test]
async fn the_root_leads_to_the_only_library() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (from, to) in [("/", "/v1"), ("/v1", "/v1/library")] {
        let req = test::TestRequest::with_uri(from)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::FOUND, "{} should redirect", from);
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            &format!("{}{}", BASE, to),
            "{} should lead to {}",
            from,
            to
        );
    }
}

// The bare paths are where OPDS 1.2 lived before 0.2.1. They still redirect there
#[test]
async fn the_paths_of_older_orcas_lead_into_v1() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for path in ["/library", "/library/books", "/library/authors/5", "/library/new",
                 "/library/opensearch.xml", "/library/search?query=alice",
                 "/library/books?page=2"] {
        let req = test::TestRequest::with_uri(path)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::FOUND, "{} should redirect", path);
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            &format!("{}/v1{}", BASE, path),
            "{} should lead into /v1 with its query intact",
            path
        );
    }

    // book covers and files did not move
    for path in ["/library/cover/5", "/library/file/4/epub"] {
        let req = test::TestRequest::with_uri(path)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert!(resp.status().is_success(), "{} should be served, not moved", path);
    }

    // A path under a reserved name -> 404
    for path in ["/v1/nonsense/at/all", "/v2/nonsense/at/all", "/v1/library/cover/5",
                 "/auth/nonsense", "/health/nonsense"] {
        let req = test::TestRequest::with_uri(path)
            .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{} should be a 404", path);
    }
}

// ------- New to the Atom feed in 0.2.1 -------

#[test]
async fn no_feed_carries_a_relative_link() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for path in ["/v1/library", "/v1/library/books", "/v1/library/new", "/v1/library/authors",
                 "/v1/library/tags", "/v1/library/series", "/v1/library/publishers",
                 "/v1/library/authors/5", "/v1/library/tags/5", "/v1/library/series/1",
                 "/v1/library/publishers/4", "/v1/library/search?query=alice"] {
        let content = body_of(&app, path, &credentials).await;
        assert!(
            !content.contains(r#"href="/"#),
            "{} carries a relative href:\n{}",
            path,
            content
        );
        assert!(!content.contains("<uri>/"), "{} carries a relative <uri>", path);
    }
}

#[test]
async fn the_library_offers_series_and_publishers() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/v1/library", &credentials).await;

    assert_eq!(
        titles(&content),
        ["All Books", "Recently Added", "Authors", "Tags", "Series", "Publishers"]
    );
    assert_eq!(count_links(&content, "/v1/library/series"), 1);
    assert_eq!(count_links(&content, "/v1/library/publishers"), 1);
}

// The volume number decides the order
#[test]
async fn a_series_stands_in_reading_order() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/v1/library/series/1", &credentials).await;

    assert_eq!(
        titles(&content),
        ["Vampires of Space", "At the Mountains of Madness", "Galactic Patrol"]
    );
}

#[test]
async fn a_publisher_holds_everything_it_put_out() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/v1/library/publishers/3", &credentials).await;
    assert_eq!(count_items(&content), 2);
    assert!(content.contains("<title>library | Gutenberg Project</title>"));

    let content = body_of(&app, "/v1/library/publishers/4", &credentials).await;
    assert_eq!(count_items(&content), 3);
}

#[test]
async fn a_shelf_feed_says_which_shelf_it_is() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for (path, title) in [
        ("/v1/library/books", "library | All Books"),
        ("/v1/library/authors/5", "library | Immanuel Kant"),
        ("/v1/library/tags/5", "library | fiction"),
        ("/v1/library/series/1", "library | Astounding Stories"),
        ("/v1/library/publishers/2", "library | Projekt Gutenberg"),
        ("/v1/library/authors", "library | Authors"),
        ("/v1/library/series", "library | Series"),
        ("/v1/library/search?query=alice", "library | Search: alice"),
    ] {
        let content = body_of(&app, path, &credentials).await;
        assert!(
            content.contains(&format!("<title>{}</title>", title)),
            "{} should be called {}",
            path,
            title
        );
    }
}

// Atom has nowhere to put a count, so a navigation entry says it in words.
#[test]
async fn a_navigation_entry_says_how_much_is_behind_it() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/v1/library/series", &credentials).await;
    assert!(content.contains("<content type=\"text\">3 books</content>"));

    // Projekt Gutenberg published one, and one book is not "1 books".
    let content = body_of(&app, "/v1/library/publishers", &credentials).await;
    assert!(content.contains("<content type=\"text\">1 book</content>"));
    assert!(content.contains("<content type=\"text\">2 books</content>"));
}

// OPDS 1.2 carries paging in OpenSearch counts and RFC 5005 links
#[test]
async fn a_paged_feed_says_where_it_stands() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/v1/library/books", &credentials).await;
    assert!(content.contains("<opensearch:totalResults>7</opensearch:totalResults>"));
    assert!(content.contains("<opensearch:itemsPerPage>50</opensearch:itemsPerPage>"));
    assert!(content.contains("<opensearch:startIndex>1</opensearch:startIndex>"));
    // One page: nowhere else to go.
    assert!(!content.contains(r#"rel="next""#));
    assert!(!content.contains(r#"rel="previous""#));

    // A shelf counts its own books, not the library's.
    let content = body_of(&app, "/v1/library/tags/5", &credentials).await;
    assert!(content.contains("<opensearch:totalResults>2</opensearch:totalResults>"));
}

#[test]
async fn a_page_past_the_end_is_the_last_page() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/v1/library/books?page=9", &credentials).await;

    assert_eq!(count_items(&content), 7);
    // The one page there is wears a bare URL.
    assert!(content.contains(&format!(r#"rel="self" href="{}/v1/library/books""#, BASE)));
}

// Recently Added is a window on the library rather than a shelf of it.
#[test]
async fn the_newest_arrivals_are_not_paged() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/v1/library/new", &credentials).await;

    assert!(!content.contains("opensearch:totalResults"));
}

#[test]
async fn an_entry_carries_the_metadata_calibre_holds() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/v1/library/authors/4", &credentials).await;

    // Alice: three tags, English, published by the Gutenberg Project in 2008.
    assert!(content.contains(r#"<category term="fiction" label="fiction"/>"#));
    assert!(content.contains(r#"<category term="children" label="children"/>"#));
    assert!(content.contains("<dcterms:language>en</dcterms:language>"));
    assert!(content.contains("<dcterms:publisher>Gutenberg Project</dcterms:publisher>"));
    assert!(content.contains("<dcterms:issued>2008-06-27T00:00:00+00:00</dcterms:issued>"));

    // Calibre has already counted the octets of every file it holds.
    assert!(link_to(&content, "/library/file/4/epub").contains(r#"length="189535""#));
    assert!(link_to(&content, "/library/file/4/azw3").contains(r#"length="256404""#));

    assert!(content.contains("<summary type=\"text\">"));
}

// what a client offers as "open in browser".
#[test]
async fn an_entry_leads_to_the_page_the_book_came_from() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/v1/library/books", &credentials).await;

    assert!(content.contains(
        r#"<link href="https://www.gutenberg.org/ebooks/70652" type="text/html" rel="alternate"/>"#
    ));

    for entry in content.split("<entry>").skip(1) {
        assert!(
            entry.matches(r#"rel="alternate""#).count() <= 1,
            "an entry may carry one alternate at most:\n{}",
            entry
        );
    }
    // Alice has no identifiers in Calibre, so: no alternate.
    let alice = body_of(&app, "/v1/library/authors/4", &credentials).await;
    assert!(!alice.contains(r#"rel="alternate""#), "{}", alice);
}

// OPDS 1.2 cannot say "volume 3 of 5", but `related` gets a client to the rest.
#[test]
async fn a_book_leads_to_its_series_and_its_publisher() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/v1/library/books", &credentials).await;

    let series = link_to(&content, "/v1/library/series/1");
    assert!(series.contains(r#"rel="related""#), "got:{}", series);
    assert!(series.contains(r#"title="Astounding Stories""#), "got:{}", series);

    let publisher = link_to(&content, "/v1/library/publishers/4");
    assert!(publisher.contains(r#"rel="related""#), "got:{}", publisher);

    // Tolstoy is in no series and nobody published him, so his entry says nothing.
    let content = body_of(&app, "/v1/library/authors/2", &credentials).await;
    assert!(!content.contains(r#"rel="related""#));
    assert!(!content.contains("dcterms:publisher"));
}

#[test]
async fn every_feed_below_the_root_leads_back_up() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for path in ["/v1/library/books", "/v1/library/new", "/v1/library/authors", "/v1/library/tags",
                 "/v1/library/series", "/v1/library/publishers", "/v1/library/authors/5",
                 "/v1/library/series/1", "/v1/library/search?query=alice"] {
        let content = body_of(&app, path, &credentials).await;
        assert!(
            content.contains(&format!(r#"rel="up" href="{}/v1/library""#, BASE)),
            "{} should lead back to the library",
            path
        );
    }

    // The library root is what they lead back to, so it has no `up` of its own.
    let content = body_of(&app, "/v1/library", &credentials).await;
    assert!(!content.contains(r#"rel="up""#));
}

// An OPDS 1.2 client finds search through an OpenSearch description
#[test]
async fn every_library_feed_advertises_its_search() {
    let app = setup(&TWO_LIBRARIES).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for path in ["/v1/library", "/v1/library/books", "/v1/library/tags", "/v1/library2"] {
        let content = body_of(&app, path, &credentials).await;
        let lib = path.split('/').nth(2).unwrap();
        assert!(
            content.contains(&format!(
                r#"rel="search" href="{}/v1/{}/opensearch.xml""#,
                BASE, lib
            )),
            "{} should advertise its search",
            path
        );
    }

    // The catalog spans libraries, and there is nothing there to search.
    let content = body_of(&app, "/v1", &credentials).await;
    assert!(!content.contains(r#"rel="search""#));
}

// `{searchTerms}` is OpenSearch's own spelling
#[test]
async fn the_opensearch_description_says_how_to_search() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let req = test::TestRequest::with_uri("/v1/library/opensearch.xml")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/opensearchdescription+xml"
    );

    let body = test::read_body(resp).await;
    let content = String::from_utf8(body.to_vec()).expect("Failed to convert to String");
    assert!(content.contains(&format!(
        r#"template="{}/v1/library/search?query={{searchTerms}}""#,
        BASE
    )));
    assert!(content.contains("<ShortName>library</ShortName>"));
}

#[test]
async fn search_finds_a_book_by_title_and_by_author() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    let content = body_of(&app, "/v1/library/search?query=alice", &credentials).await;
    assert_eq!(count_items(&content), 1);
    assert_eq!(titles(&content), ["Alice&#39;s Adventures in Wonderland"]);

    // Kant wrote one book in this library, and nothing has him in the title.
    let content = body_of(&app, "/v1/library/search?query=kant", &credentials).await;
    assert_eq!(titles(&content), ["Kritik der reinen Vernunft - 2. Auflage"]);
}

// A bare `%%` / empty search field should not match the whole library
#[test]
async fn an_empty_search_finds_nothing() {
    let app = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");

    for path in ["/v1/library/search", "/v1/library/search?query="] {
        let content = body_of(&app, path, &credentials).await;
        assert_eq!(count_items(&content), 0, "{} should find nothing", path);
        assert!(content.contains("<title>library | Search</title>"));
    }
}

// ------- Two libraries -------

// A root with something to choose from is a catalog rather than a redirect.
#[test]
async fn multiple_libraries_are_listed() {
    let app = setup(&TWO_LIBRARIES).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let content = body_of(&app, "/v1", &credentials).await;

    assert!(is_opds(&content));
    assert_eq!(count_items(&content), 2);
    assert!(content.contains("<title>library</title>"));
    assert!(content.contains("<title>library2</title>"));
    assert_eq!(count_links(&content, "/v1/library2"), 1);
}

#[test]
async fn unauthorized_request_https() {
    let app = setup(&TWO_LIBRARIES).await;
    let req = test::TestRequest::with_uri("/v1/library")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[test]
async fn unauthorized_request_public_route_https() {
    let app = setup(&TWO_LIBRARIES).await;
    let req = test::TestRequest::with_uri("/health")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
}

#[test]
async fn authorized_request_https() {
    let app = setup(&TWO_LIBRARIES).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/v1/library")
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
    let app = setup(&TWO_LIBRARIES).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let req = test::TestRequest::with_uri("/health")
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
}

// ------- A landing page with named shelves on it -------

/// The shelves in `tests/shelves.toml`, against the tags in `tests/calibre`:
///   science fiction  2, 7, 8, 9      fantasy  4
///   horror           8               fiction  2, 4
#[test]
async fn a_landing_page_offers_the_sections_it_names() {
    let app = setup(&SHELVES).await;
    let content = body_of(&app, "/v1/library", "").await;

    // no book in the library is tagged "chess". The ampersand is escaped.
    assert_eq!(
        titles(&content),
        ["All Books", "Scifi &amp; Fantasy", "Cosmic Horror", "New Arrivals in Fiction", "Genres"]
    );
    assert_eq!(count_links(&content, "/v1/library/shelves/scifi"), 1);
    assert!(!content.contains("/shelves/chess"), "{}", content);

    // A filtered section says which books it holds; they are not counted.
    assert!(
        content.contains("<content type=\"text\">Books tagged Science Fiction or fantasy</content>"),
        "{}",
        content
    );
}

#[test]
async fn a_named_shelf_holds_the_books_carrying_any_of_its_tags() {
    let app = setup(&SHELVES).await;
    let content = body_of(&app, "/v1/library/shelves/scifi", "").await;

    assert_eq!(count_items(&content), 5);
    assert!(content.contains("<title>library | Scifi &amp; Fantasy</title>"), "{}", content);
}

#[test]
async fn tags_all_wants_every_one_of_them_and_not_merely_one() {
    let app = setup(&SHELVES).await;
    let content = body_of(&app, "/v1/library/shelves/cosmic-horror", "").await;

    // Only "At the Mountains of Madness" is both.
    assert_eq!(count_items(&content), 1);
    assert!(content.contains("At the Mountains of Madness"), "{}", content);
}

// A `new` section is a window on its shelf: first filtered, then capped.
#[test]
async fn a_filtered_new_section_holds_the_newest_of_the_books_that_match() {
    let app = setup(&SHELVES).await;
    let content = body_of(&app, "/v1/library/shelves/fresh-fiction", "").await;

    assert_eq!(count_items(&content), 2);
    // Unpaged, like `/new`
    assert!(!content.contains("rel=\"next\""), "{}", content);
}

#[test]
async fn a_named_shelf_that_matches_nothing_is_an_empty_feed_and_not_a_404() {
    let app = setup(&SHELVES).await;
    let content = body_of(&app, "/v1/library/shelves/chess", "").await;

    assert_eq!(count_items(&content), 0);
    assert!(content.contains("<title>library | Chess</title>"), "{}", content);
}

// A slug only exists if it is defined on a landing page
#[test]
async fn a_slug_no_landing_page_defines_is_not_found() {
    let app = setup(&SHELVES).await;
    let (status, _) = answer_to(&app, "/v1/library/shelves/nonesuch", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // ...and the default landing page defines none at all.
    let plain = setup(&ONE_LIBRARY).await;
    let credentials = BASE64.encode("alice:secretpassword");
    let (status, _) = answer_to(&plain, "/v1/library/shelves/scifi", &credentials).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[test]
async fn a_named_shelf_pages_like_every_other_shelf() {
    let app = setup(&SHELVES).await;
    let content = body_of(&app, "/v1/library/shelves/scifi?page=2", "").await;

    // Five books fit on one page, so page two is page one.
    assert_eq!(count_items(&content), 5);
    assert!(content.contains(r#"opensearch:totalResults>5<"#), "{}", content);
}

// Atom has no way of rendering book covers directly on the page, so `show` is a v2 thing
#[test]
async fn a_section_that_shows_its_books_is_an_ordinary_link_in_atom() {
    let app = setup(&INLINE).await;
    let content = body_of(&app, "/v1/library", "").await;

    assert_eq!(
        titles(&content),
        ["Recently Added", "All Books", "Scifi &amp; Fantasy", "Book of the Month", "Authors"]
    );
    assert_eq!(count_links(&content, "/v1/library/shelves/scifi"), 1);
    assert!(!content.contains("/shelves/chess"), "{}", content);

    // Every entry is a section and none of them is a book: there is no acquisition link among them.
    assert_eq!(count_items(&content), 5);
    assert!(!content.contains("opds-spec.org/acquisition"), "{}", content);
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

/// The status a URI answers with, and where it points if it points anywhere.
/// For the feeds that redirect, where `body_of` would only assert success.
async fn answer_to(
    app: &impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    uri: &str,
    credentials: &str,
) -> (StatusCode, Option<String>) {
    let req = test::TestRequest::with_uri(uri)
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    let resp = test::call_service(app, req).await;
    let location = resp
        .headers()
        .get(header::LOCATION)
        .map(|value| value.to_str().expect("Location is not valid UTF-8").to_string());

    (resp.status(), location)
}

fn count_links(content: &str, path: &str) -> usize {
    content.matches(&format!(r#"href="{}{}""#, BASE, path)).count()
}

fn link_to(content: &str, path: &str) -> String {
    content
        .split(&format!("href=\"{}{}\"", BASE, path))
        .nth(1)
        .unwrap_or_else(|| panic!("nothing links to {}", path))
        .split("/>")
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The text of every `<tag>` in a feed, in the order it appears.
fn values_of(content: &str, tag: &str) -> Vec<String> {
    content
        .split(&format!("<{}>", tag))
        .skip(1)
        .map(|tail| tail.split('<').next().unwrap_or_default().to_string())
        .collect()
}

/// The titles of the entries of a feed, in the order they are listed.
fn titles(content: &str) -> Vec<String> {
    // The feed's own <title> comes first
    values_of(content, "title").into_iter().skip(1).collect()
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
