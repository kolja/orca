//! OPDS 2.0 feeds, checked against the spec
//!
//! schemas live in `tests/schema`, fetched by `scripts/fetch-schemas.sh`:
//! the two OPDS ones pull in a lot of Readium schemas

use actix_http::Request;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{header, StatusCode};
use actix_web::{test, web, App};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use boon::{Compiler, Schemas};
use once_cell::sync::Lazy;
use orca::config::{read_config, Config};
use orca::{create_app, init};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const FEED: &str = "https://specs.opds.io/schema/feed.schema.json";
const PUBLICATION: &str = "https://specs.opds.io/schema/publication.schema.json";

static TEST_HTTP_CONFIG: Lazy<Config> =
    Lazy::new(|| read_config("tests/orca.http.test.toml").expect("Failed to read test config"));

// Two libraries, so the catalog root has something to list.
static TEST_HTTPS_CONFIG: Lazy<Config> =
    Lazy::new(|| read_config("tests/orca.https.test.toml").expect("Failed to read test config"));

// ------- The feeds -------

// One library: the root is a redirect, the same as the OPDS 1.2 root.
#[test]
async fn a_single_library_needs_no_catalog() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let response = call_authorized(&app, "/v2").await;

    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/v2/library");
}

#[test]
async fn the_catalog_lists_every_library() {
    let app = setup(&TEST_HTTPS_CONFIG).await;
    let catalog = feed(&app, "/v2").await;

    validates(&catalog, FEED);
    assert_eq!(catalog["metadata"]["title"], "ORCA");
    assert_eq!(titles(&catalog["navigation"]), ["library", "library2"]);
}

#[test]
async fn a_library_offers_its_books() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let library = feed(&app, "/v2/library").await;

    validates(&library, FEED);
    assert_eq!(library["metadata"]["title"], "library");
    assert_eq!(titles(&library["navigation"]), ["All Books", "Recently Added"]);
    assert_eq!(library["navigation"][0]["properties"]["numberOfItems"], 7);
}

#[test]
async fn every_book_feed_is_a_valid_feed() {
    let app = setup(&TEST_HTTP_CONFIG).await;

    for path in ["/v2/library/books", "/v2/library/new"] {
        let books = feed(&app, path).await;
        validates(&books, FEED);
        assert_eq!(books["publications"].as_array().unwrap().len(), 7);
    }
}

#[test]
async fn a_publication_is_valid_on_its_own() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let response = call_authorized(&app, "/v2/library/book/4").await;

    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/opds-publication+json"
    );
    validates(&json(response).await, PUBLICATION);
}

#[test]
async fn a_book_the_library_does_not_hold_is_404() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    assert_eq!(call_authorized(&app, "/v2/library/book/99999").await.status(), StatusCode::NOT_FOUND);
}

#[test]
async fn a_library_that_is_not_there_is_404() {
    let app = setup(&TEST_HTTP_CONFIG).await;

    for path in ["/v2/nope", "/v2/nope/books", "/v2/nope/new", "/v2/nope/book/4"] {
        assert_eq!(call_authorized(&app, path).await.status(), StatusCode::NOT_FOUND, "{}", path);
    }
}

#[test]
async fn opds2_is_no_more_public_than_opds1() {
    let app = setup(&TEST_HTTP_CONFIG).await;

    for path in ["/v2", "/v2/library", "/v2/library/books", "/v2/library/book/4"] {
        assert_eq!(call(&app, path).await.status(), StatusCode::UNAUTHORIZED, "{}", path);
    }
}

// ------- What a publication says -------

#[test]
async fn a_publication_carries_the_metadata_calibre_holds() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let alice = publication(&app, 4).await;

    assert_eq!(alice["metadata"]["@type"], "http://schema.org/Book");
    assert_eq!(alice["metadata"]["title"], "Alice's Adventures in Wonderland");
    assert_eq!(alice["metadata"]["author"][0]["name"], "Lewis Carroll");
    // Calibre stores ISO 639-2, OPDS clients read BCP 47.
    assert_eq!(alice["metadata"]["language"][0], "en");
    // An identifier that stays the same wherever the book turns up.
    assert_eq!(
        alice["metadata"]["identifier"],
        "urn:uuid:8b9f853c-7171-4f09-ba21-7304603a5128"
    );
    assert_eq!(alice["images"][0]["type"], "image/jpeg");
}

#[test]
async fn a_publication_carries_the_shelf_it_came_off() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let patrol = publication(&app, 9).await;

    validates(&patrol, PUBLICATION);
    let metadata = &patrol["metadata"];
    assert_eq!(metadata["publisher"], "Street & Smith");
    assert_eq!(metadata["subject"], json!(["science fiction", "space opera"]));
    assert_eq!(metadata["belongsTo"]["series"]["name"], "Astounding Stories");
    // Third of the three Astounding Stories in the library.
    assert_eq!(metadata["belongsTo"]["series"]["position"], 3.0);
}

// no empty `belongsTo`
#[test]
async fn a_book_on_no_shelf_belongs_to_nothing() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let alice = publication(&app, 4).await;

    assert!(alice["metadata"].get("belongsTo").is_none());
    assert!(alice["metadata"].get("publisher").is_some());
    assert_eq!(alice["metadata"]["subject"], json!(["children", "fantasy", "fiction"]));
}

// The Atom feed wraps a blurb at 100 columns to fit `<content type="text">`.
// In JSON the same blurb keeps only the breaks its author wrote.
#[test]
async fn a_description_is_not_hard_wrapped() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let alice = publication(&app, 4).await;
    let description = alice["metadata"]["description"].as_str().expect("description");

    assert!(
        description.lines().any(|line| line.chars().count() > 100),
        "the blurb still looks wrapped to 100 columns: {}",
        description
    );
}

// a publication with no acquisition link fails the schema:
#[test]
async fn every_format_is_an_acquisition_link() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let alice = publication(&app, 4).await;

    let acquisitions: Vec<&Value> = alice["links"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|link| link["rel"] == "http://opds-spec.org/acquisition")
        .collect();

    assert_eq!(acquisitions.len(), 2);
    // Sorted, so that the download links of a book do not shuffle between requests.
    assert_eq!(acquisitions[0]["type"], "application/vnd.amazon.mobi8-ebook");
    assert_eq!(acquisitions[1]["type"], "application/epub+zip");
}

// OPDS v1.2 and v2 clients downloads through the same route.
#[test]
async fn what_a_publication_links_to_can_be_downloaded() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let alice = publication(&app, 4).await;

    let mut followed = 0;
    for link in alice["links"].as_array().unwrap().iter().chain(alice["images"].as_array().unwrap()) {
        let href = link["href"].as_str().expect("href");
        assert!(href.starts_with("http://"), "{} is not absolute", href);

        let path = href.strip_prefix("http://localhost:8080").expect("this server");
        assert!(call_authorized(&app, path).await.status().is_success(), "{}", href);
        followed += 1;
    }
    // self, two acquisitions, one cover.
    assert_eq!(followed, 4);
}

// ------- Paging -------

#[test]
async fn a_library_that_fits_on_one_page_links_to_no_other() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let books = feed(&app, "/v2/library/books").await;

    assert_eq!(books["metadata"]["numberOfItems"], 7);
    assert_eq!(books["metadata"]["itemsPerPage"], 50);
    assert_eq!(books["metadata"]["currentPage"], 1);
    assert_eq!(rels(&books["links"]), ["self", "start"]);
}

// A client following a `next` link it kept from before the library shrank.
#[test]
async fn a_page_past_the_end_still_holds_books() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let books = feed(&app, "/v2/library/books?page=9").await;

    validates(&books, FEED);
    assert_eq!(books["metadata"]["currentPage"], 1);
    assert_eq!(books["publications"].as_array().unwrap().len(), 7);
    // The feed says where it really is, not where it was asked to be.
    assert_eq!(books["links"][0]["href"], "http://localhost:8080/v2/library/books");
}

// does the schema catch a feed without a `self` link
#[test]
async fn the_schema_check_has_teeth() {
    let feed = serde_json::json!({
        "metadata": { "title": "library" },
        "links": [{ "href": "http://localhost:8080/v2/library" }],
        "navigation": [{ "href": "http://localhost:8080/v2/library/books", "title": "All Books" }],
    });

    assert!(validate(&feed, FEED).is_err());
}

// ------- OPDS 1.2 is untouched -------

// `/v2` is registered before `/{lib}`, which would otherwise swallow it.
#[test]
async fn the_atom_catalog_still_answers() {
    let app = setup(&TEST_HTTP_CONFIG).await;
    let response = call_authorized(&app, "/library/books").await;

    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/atom+xml"
    );
}

// ------- Helper Functions -------

async fn setup(
    config: &'static Config,
) -> impl Service<Request, Response = ServiceResponse, Error = actix_web::Error> {
    let state = create_app(config).expect("Failed to create app");
    test::init_service(App::new().app_data(web::Data::new(state)).configure(init)).await
}

async fn call(
    app: &impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    uri: &str,
) -> ServiceResponse {
    test::call_service(app, test::TestRequest::with_uri(uri).to_request()).await
}

async fn call_authorized(
    app: &impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    uri: &str,
) -> ServiceResponse {
    let credentials = BASE64.encode("alice:secretpassword");
    let request = test::TestRequest::with_uri(uri)
        .insert_header((header::AUTHORIZATION, format!("Basic {}", credentials)))
        .to_request();
    test::call_service(app, request).await
}

async fn json(response: ServiceResponse) -> Value {
    let body = test::read_body(response).await;
    serde_json::from_slice(&body).expect("a JSON body")
}

/// One feed, which has to have been served as one.
async fn feed(
    app: &impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    uri: &str,
) -> Value {
    let response = call_authorized(app, uri).await;
    assert!(response.status().is_success(), "{} should succeed", uri);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/opds+json",
        "{}",
        uri
    );
    json(response).await
}

async fn publication(
    app: &impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    id: i32,
) -> Value {
    json(call_authorized(app, &format!("/v2/library/book/{}", id)).await).await
}

fn titles(links: &Value) -> Vec<String> {
    collect(links, "title")
}

fn rels(links: &Value) -> Vec<String> {
    collect(links, "rel")
}

fn collect(links: &Value, key: &str) -> Vec<String> {
    links
        .as_array()
        .expect("a collection of links")
        .iter()
        .map(|link| link[key].as_str().unwrap_or_default().to_string())
        .collect()
}

fn validates(json: &Value, schema: &str) {
    if let Err(complaint) = validate(json, schema) {
        panic!("{}\n{}", serde_json::to_string_pretty(json).unwrap(), complaint);
    }
}

/// Check one feed or publication against the schema it claims to be.
///
/// No loader is registered, so a `$ref` that was never vendored fails the test
/// instead of quietly going out to the network.
fn validate(json: &Value, schema: &str) -> Result<(), String> {
    let mut compiler = Compiler::new();
    for (url, definition) in vendored_schemas() {
        compiler.add_resource(&url, definition).expect("a schema resource");
    }

    let mut schemas = Schemas::new();
    let index = compiler.compile(schema, &mut schemas).expect("a compiled schema");

    schemas.validate(json, index).map_err(|e| format!("{:#}", e))
}

/// Every schema under `tests/schema`, keyed by the URL it was fetched from --
/// which is the `$id` the other schemas refer to it by.
fn vendored_schemas() -> Vec<(String, Value)> {
    let root = Path::new("tests/schema");
    let mut files = Vec::new();
    walk(root, &mut files);
    assert!(!files.is_empty(), "run scripts/fetch-schemas.sh");

    files
        .iter()
        .map(|file| {
            let url = file.strip_prefix(root).expect("below tests/schema");
            let definition = fs::read_to_string(file).expect("a readable schema");
            (
                format!("https://{}", url.to_string_lossy()),
                serde_json::from_str(&definition).expect("a schema that is JSON"),
            )
        })
        .collect()
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("tests/schema").flatten() {
        match entry.path() {
            path if path.is_dir() => walk(&path, files),
            path if path.extension().is_some_and(|kind| kind == "json") => files.push(path),
            _ => (),
        }
    }
}
