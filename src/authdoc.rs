
//! The OPDS Authentication Document
//!
//! <https://drafts.opds.io/authentication-for-opds-1.0>
//! What a 401 answers with: which catalog is asking, and how to tell it who you are.

use serde_derive::Serialize;

use crate::config::CATALOG;
use crate::opds2::Link;

/// The media type of the document / body of every 401
pub const AUTHENTICATION: &str = "application/opds-authentication+json";

pub const BASIC: &str = "http://opds-spec.org/auth/basic";

pub const PATH: &str = "/auth";

/// The media type of the logo a client shows with its login form
pub const LOGO: &str = "image/svg+xml";

#[derive(Debug, Serialize)]
pub struct AuthDocument {
    pub id: String,
    pub title: &'static str,
    pub authentication: Vec<Authentication>,
    pub links: Vec<Link>,
}

#[derive(Debug, Serialize)]
pub struct Authentication {
    /// `type` in JSON, is a keyword in Rust.
    #[serde(rename = "type")]
    pub kind: &'static str,
}

/// The document Orca serves at `/auth` and refuses every request with.
///
/// no `labels`: they  would replace the client's own translated "Username" and "Password"
pub fn document(origin: &str) -> AuthDocument {
    let url = format!("{}{}", origin, PATH);

    AuthDocument {
        id: url.clone(),
        title: CATALOG,
        authentication: vec![Authentication { kind: BASIC }],
        links: vec![
            Link::new(url).rel("self").mime(AUTHENTICATION),
            Link::new(format!("{}{}/logo.svg", origin, PATH)).rel("logo").mime(LOGO),
        ],
    }
}

pub fn body(origin: &str) -> String {
    serde_json::to_string(&document(origin)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    #[test]
    fn the_document_says_who_asks_and_how_to_answer() {
        assert_eq!(
            to_value(document("https://books.example")).unwrap(),
            json!({
                "id": "https://books.example/auth",
                "title": "ORCA",
                "authentication": [{ "type": "http://opds-spec.org/auth/basic" }],
                "links": [
                    {
                        "href": "https://books.example/auth",
                        "rel": "self",
                        "type": "application/opds-authentication+json",
                    },
                    {
                        "href": "https://books.example/auth/logo.svg",
                        "rel": "logo",
                        "type": "image/svg+xml",
                    },
                ],
            })
        );
    }

    // Whatever the client calls a login, in whatever language it was installed in.
    #[test]
    fn the_labels_are_left_to_the_client() {
        let document = to_value(document("https://books.example")).unwrap();

        assert!(document["authentication"][0].get("labels").is_none());
    }
}
