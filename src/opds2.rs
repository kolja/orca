
//! The OPDS 2.0 wire format
//!
//! One serde type per object of <https://specs.opds.io/opds-2.0>
//! Everything in here is agnostic of Calibre
//! optional keys are left out rather than serialised as `null` or `[]`

use serde_derive::Serialize;

/// The media type of a feed or the type of a link that leads to one
pub const FEED: &str = "application/opds+json";

/// A publication served on its own, outside of any feed.
pub const PUBLICATION: &str = "application/opds-publication+json";

/// What a publication *is*, for the `@type` of its metadata.
pub const BOOK: &str = "http://schema.org/Book";

/// The rel of a link that leads to the book itself.
pub const ACQUISITION: &str = "http://opds-spec.org/acquisition";

/// The rel of a feed of what arrived last.
pub const SORT_NEW: &str = "http://opds-spec.org/sort/new";

/// A feed is `metadata` and `links` plus at least one of `navigation`,
/// `publications` or `groups`: a catalog of places to go, or of books to read.
#[derive(Debug, Serialize)]
pub struct Feed {
    pub metadata: FeedMetadata,
    pub links: Vec<Link>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub navigation: Vec<Link>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub publications: Vec<Publication>,
}

impl Feed {
    /// A feed knows its own address
    pub fn new(title: impl Into<String>, self_url: impl Into<String>) -> Self {
        Feed {
            metadata: FeedMetadata {
                title: title.into(),
                ..FeedMetadata::default()
            },
            links: vec![Link::new(self_url).rel("self").mime(FEED)],
            navigation: Vec::new(),
            publications: Vec::new(),
        }
    }

    pub fn modified(mut self, when: impl Into<String>) -> Self {
        self.metadata.modified = Some(when.into());
        self
    }

    pub fn navigation(mut self, links: Vec<Link>) -> Self {
        self.navigation = links;
        self
    }

    pub fn publications(mut self, publications: Vec<Publication>) -> Self {
        self.publications = publications;
        self
    }

    pub fn link(mut self, link: Link) -> Self {
        self.links.push(link);
        self
    }

    /// Where in the catalog this feed sits, for a client that wants to show
    /// "page 2 of 7" without counting the links itself.
    pub fn page(mut self, total: usize, per_page: usize, current: usize) -> Self {
        self.metadata.number_of_items = Some(total);
        self.metadata.items_per_page = Some(per_page);
        self.metadata.current_page = Some(current);
        self
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedMetadata {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_items: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_per_page: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_page: Option<usize>,
}

/// A Link Object: for navigation entries, acquisition links, cover images and pagination
#[derive(Debug, Serialize)]
pub struct Link {
    pub href: String,
    /// `type` in JSON, is a keyword in Rust.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Properties>,
}

impl Link {
    pub fn new(href: impl Into<String>) -> Self {
        Link {
            href: href.into(),
            mime: None,
            rel: None,
            title: None,
            properties: None,
        }
    }

    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    pub fn rel(mut self, rel: impl Into<String>) -> Self {
        self.rel = Some(rel.into());
        self
    }

    /// Required of every link in a `navigation` collection, and what a client
    /// puts on the download button of an acquisition link.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// How many publications wait behind this link. A navigation entry may say
    /// so, which spares the client a request to find out.
    pub fn count(mut self, items: usize) -> Self {
        self.properties = Some(Properties {
            number_of_items: items,
        });
        self
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Properties {
    pub number_of_items: usize,
}

/// One book: what it is, where to get it, what it looks like.
#[derive(Debug, Serialize)]
pub struct Publication {
    pub metadata: BookMetadata,
    /// At least one of these has to be an acquisition link
    pub links: Vec<Link>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Link>,
}

#[derive(Debug, Serialize)]
pub struct BookMetadata {
    #[serde(rename = "@type")]
    pub kind: &'static str,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub author: Vec<Contributor>,
    /// BCP 47 tags; the schema checks them against the full grammar.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub language: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A contributor may just be a string, but the object could add a link to the author's own feed.
#[derive(Debug, Serialize)]
pub struct Contributor {
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    #[test]
    fn a_feed_knows_where_it_lives() {
        let feed = Feed::new("library", "https://books.example/v2/library");

        assert_eq!(
            to_value(&feed).unwrap(),
            json!({
                "metadata": { "title": "library" },
                "links": [{
                    "href": "https://books.example/v2/library",
                    "rel": "self",
                    "type": "application/opds+json",
                }],
            })
        );
    }

    // `links: []` is rejected by the schema
    #[test]
    fn what_is_empty_is_left_out() {
        let feed = to_value(Feed::new("library", "/v2/library")).unwrap();

        assert!(feed.get("navigation").is_none());
        assert!(feed.get("publications").is_none());
        assert!(feed["metadata"].get("modified").is_none());
        assert!(feed["links"][0].get("title").is_none());
    }

    #[test]
    fn a_navigation_entry_can_say_how_much_is_behind_it() {
        let link = Link::new("/v2/library/books")
            .rel("subsection")
            .mime(FEED)
            .title("All Books")
            .count(1234);

        assert_eq!(
            to_value(&link).unwrap(),
            json!({
                "href": "/v2/library/books",
                "rel": "subsection",
                "type": "application/opds+json",
                "title": "All Books",
                "properties": { "numberOfItems": 1234 },
            })
        );
    }

    #[test]
    fn pagination_is_metadata_as_well_as_links() {
        let feed = Feed::new("library", "/v2/library/books?page=2")
            .page(120, 50, 2)
            .link(Link::new("/v2/library/books").rel("previous").mime(FEED));

        let json = to_value(&feed).unwrap();
        assert_eq!(json["metadata"]["numberOfItems"], 120);
        assert_eq!(json["metadata"]["itemsPerPage"], 50);
        assert_eq!(json["metadata"]["currentPage"], 2);
        assert_eq!(json["links"][1]["rel"], "previous");
    }
}
