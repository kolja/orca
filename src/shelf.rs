
//! A `Shelf` says how to ask Calibre and what to call the answer.
//! It is shared for OPDS v1 and v2 (XML / JSON) feeds.

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use rusqlite::Connection;
use serde_derive::Serialize;

use crate::calibre::{self, Book};

/// How many books one page of a feed holds.
pub const PER_PAGE: usize = 50;

/// a `Shelf` carries no books. Its a query.
pub enum Shelf {
    Everything,
    Author(i32),
    Tag(i32),
    Series(i32),
    Publisher(i32),
    /// a term nothing matches is an empty shelf, not a 404.
    Search(String),
}

impl Shelf {
    pub fn feed(&self) -> &'static str {
        match self {
            Shelf::Everything => "books",
            Shelf::Author(_) => "authors",
            Shelf::Tag(_) => "tags",
            Shelf::Series(_) => "series",
            Shelf::Publisher(_) => "publishers",
            Shelf::Search(_) => "search",
        }
    }

    pub fn noun(&self) -> &'static str {
        match self {
            Shelf::Everything => "books",
            Shelf::Author(_) => "author",
            Shelf::Tag(_) => "tag",
            Shelf::Series(_) => "series",
            Shelf::Publisher(_) => "publisher",
            Shelf::Search(_) => "search",
        }
    }

    pub fn path(&self) -> String {
        match self {
            Shelf::Everything => self.feed().to_string(),
            Shelf::Author(id) | Shelf::Tag(id) | Shelf::Series(id) | Shelf::Publisher(id) => {
                format!("{}/{}", self.feed(), id)
            }
            // The term is part of the address, so every page of a search finds
            // its way back to the same books.
            Shelf::Search(term) => format!("{}?query={}", self.feed(), encoded(term)),
        }
    }

    /// What to call this feed. `QueryReturnedNoRows` for a shelf the library does not have.
    pub fn name(&self, db: &Connection) -> rusqlite::Result<String> {
        match self {
            Shelf::Everything => Ok("All Books".to_string()),
            Shelf::Author(id) => calibre::author_name(db, *id),
            Shelf::Tag(id) => calibre::tag_name(db, *id),
            Shelf::Series(id) => calibre::series_name(db, *id),
            Shelf::Publisher(id) => calibre::publisher_name(db, *id),
            Shelf::Search(term) => Ok(match term.trim() {
                "" => "Search".to_string(),
                term => format!("Search: {}", term),
            }),
        }
    }

    pub fn count(&self, db: &Connection) -> rusqlite::Result<usize> {
        match self {
            Shelf::Everything => calibre::count_books(db),
            Shelf::Author(id) => calibre::count_books_by_author(db, *id),
            Shelf::Tag(id) => calibre::count_books_by_tag(db, *id),
            Shelf::Series(id) => calibre::count_books_in_series(db, *id),
            Shelf::Publisher(id) => calibre::count_books_by_publisher(db, *id),
            Shelf::Search(term) if term.trim().is_empty() => Ok(0),
            Shelf::Search(term) => calibre::count_books_matching(db, term.trim()),
        }
    }

    /// Every shelf is in the order the whole library is in;
    /// except the series, which is in the order it is meant to be read.
    pub fn books(
        &self,
        db: &Connection,
        limit: usize,
        offset: usize,
    ) -> rusqlite::Result<Vec<Book>> {
        match self {
            Shelf::Everything => calibre::books_page(db, limit, offset),
            Shelf::Author(id) => calibre::books_by_author_page(db, *id, limit, offset),
            Shelf::Tag(id) => calibre::books_by_tag_page(db, *id, limit, offset),
            Shelf::Series(id) => calibre::books_in_series_page(db, *id, limit, offset),
            Shelf::Publisher(id) => calibre::books_by_publisher_page(db, *id, limit, offset),
            // Nothing typed is nothing found: a bare `%%` would be the whole library.
            Shelf::Search(term) if term.trim().is_empty() => Ok(Vec::new()),
            Shelf::Search(term) => calibre::books_search_page(db, term.trim(), limit, offset),
        }
    }
}

/// The feed a kind of shelf lives in: `authors` for `Shelf::Author`
pub fn feed_of(shelf: fn(i32) -> Shelf) -> &'static str {
    shelf(0).feed()
}

/// The singular a kind of shelf goes by: `author` for `Shelf::Author`
pub fn noun_of(shelf: fn(i32) -> Shelf) -> &'static str {
    shelf(0).noun()
}

/// `NON_ALPHANUMERIC` keeps `&`, `=` and `+` out of a URL
fn encoded(term: &str) -> String {
    utf8_percent_encode(term, NON_ALPHANUMERIC).to_string()
}

/// Which slice of the library a request for `page` asks for.
/// Pages count from one, and one past the end is the last page: An empty library still has a page one.
pub fn window(total: usize, per_page: usize, requested: usize) -> Window {
    let last = total.div_ceil(per_page).max(1);
    let current = requested.clamp(1, last);
    Window {
        current,
        last,
        offset: (current - 1) * per_page,
    }
}

pub struct Window {
    pub current: usize,
    pub last: usize,
    pub offset: usize,
}

/// join with whichever separator is still free (there may already be a query, e.g. search)
/// `root` is the library the feed belongs to: `https://books.example/v2/library`
pub fn page_url(root: &str, feed: &str, page: usize) -> String {
    let url = format!("{}/{}", root, feed);
    match page {
        1 => url,
        n => {
            let separator = match feed.contains('?') {
                true => '&',
                false => '?',
            };
            format!("{}{}page={}", url, separator, n)
        }
    }
}

/// link from a feed its neighbour
#[derive(Debug, Serialize)]
pub struct PageLink {
    pub rel: &'static str,
    pub href: String,
}

pub fn page_links(root: &str, feed: &str, window: &Window) -> Vec<PageLink> {
    let sibling = |rel: &'static str, number: usize| PageLink {
        rel,
        href: page_url(root, feed, number),
    };

    let mut links = Vec::new();
    if window.current > 1 {
        links.push(sibling("first", 1));
        links.push(sibling("previous", window.current - 1));
    }
    if window.current < window.last {
        links.push(sibling("next", window.current + 1));
        links.push(sibling("last", window.last));
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "https://books.example/v2/lib";

    #[test]
    fn a_library_that_fits_on_one_page_has_only_that_page() {
        let one = window(4, 50, 1);
        assert_eq!((one.current, one.last, one.offset), (1, 1, 0));

        // A library with nothing in it is still a page, not a 404.
        let empty = window(0, 50, 1);
        assert_eq!((empty.current, empty.last, empty.offset), (1, 1, 0));
    }

    #[test]
    fn pages_cover_the_library_exactly() {
        // 120 books, 50 to a page: 50 + 50 + 20.
        assert_eq!(window(120, 50, 1).offset, 0);
        assert_eq!(window(120, 50, 2).offset, 50);
        assert_eq!(window(120, 50, 3).offset, 100);
        assert_eq!(window(120, 50, 3).last, 3);

        // A page that divides evenly does not add an empty one at the end.
        assert_eq!(window(100, 50, 1).last, 2);
    }

    // A `next` link a client kept from before half the library was deleted.
    #[test]
    fn a_page_past_the_end_is_the_last_page() {
        let past = window(120, 50, 99);
        assert_eq!((past.current, past.last, past.offset), (3, 3, 100));
    }

    // `?page=0` should not underflow.
    #[test]
    fn a_page_before_the_beginning_is_the_first_page() {
        assert_eq!(window(120, 50, 0).current, 1);
        assert_eq!(window(120, 50, 0).offset, 0);
    }

    // The fixture library fits on one page, so this is where the way from one
    // page to the next is checked.
    #[test]
    fn a_page_links_to_the_pages_around_it() {
        let rels = |total, requested| {
            page_links(ROOT, "books", &window(total, 50, requested))
                .iter()
                .map(|link| link.rel)
                .collect::<Vec<_>>()
        };

        assert_eq!(rels(120, 2), ["first", "previous", "next", "last"]);
        assert_eq!(rels(120, 1), ["next", "last"]);
        assert_eq!(rels(120, 3), ["first", "previous"]);
        // Nowhere else to go.
        assert!(rels(4, 1).is_empty());
    }

    #[test]
    fn the_pages_around_a_page_are_the_ones_beside_it() {
        let links = page_links(ROOT, "books", &window(120, 50, 2));
        let href = |n: usize| links[n].href.as_str();

        assert_eq!(href(0), "https://books.example/v2/lib/books");
        assert_eq!(href(1), "https://books.example/v2/lib/books");
        assert_eq!(href(2), "https://books.example/v2/lib/books?page=3");
        assert_eq!(href(3), "https://books.example/v2/lib/books?page=3");
    }

    #[test]
    fn only_the_first_page_wears_a_bare_url() {
        assert_eq!(page_url(ROOT, "books", 1), "https://books.example/v2/lib/books");
        assert_eq!(page_url(ROOT, "books", 2), "https://books.example/v2/lib/books?page=2");
    }

    // A shelf is paged the same way the library is, and its pages are pages of
    // the shelf rather than of the catalog.
    #[test]
    fn a_shelf_pages_under_its_own_address() {
        let path = Shelf::Author(5).path();
        assert_eq!(path, "authors/5");
        assert_eq!(Shelf::Tag(9).path(), "tags/9");
        assert_eq!(Shelf::Everything.path(), "books");

        assert_eq!(page_url(ROOT, &path, 2), "https://books.example/v2/lib/authors/5?page=2");
    }

    // The Atom feeds: `/{lib}` for JSON its: `/v2/{lib}`
    // both name a shelf the same way
    #[test]
    fn the_same_shelf_pages_under_either_feed() {
        let path = Shelf::Series(1).path();
        assert_eq!(page_url("https://books.example/lib", &path, 2), "https://books.example/lib/series/1?page=2");
        assert_eq!(page_url("https://books.example/v2/lib", &path, 2), "https://books.example/v2/lib/series/1?page=2");
    }

    // A navigation entry is identified by what it points at, in the singular.
    #[test]
    fn a_shelf_is_named_in_the_singular_and_shelved_in_the_plural() {
        assert_eq!((feed_of(Shelf::Author), noun_of(Shelf::Author)), ("authors", "author"));
        assert_eq!((feed_of(Shelf::Tag), noun_of(Shelf::Tag)), ("tags", "tag"));
        assert_eq!((feed_of(Shelf::Publisher), noun_of(Shelf::Publisher)), ("publishers", "publisher"));
        // The one that is already a plural.
        assert_eq!((feed_of(Shelf::Series), noun_of(Shelf::Series)), ("series", "series"));
    }
}
