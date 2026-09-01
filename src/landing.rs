
//! What the root of a library offers.
//!
//! A landing page is a list of `[[section]]`, each one a shelf plus how to
//! present it. It is read once, at startup, and shared by both feed formats:
//! `/v1/{lib}` as atom and `/v2/{lib}` as navigation links.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::de::{self, Deserializer};
use serde_derive::Deserialize;

use rusqlite::Connection;

use crate::calibre::{self, Book, Counts};
use crate::shelf::{Named, Shelf};

/// SQLite's `NOCASE` folds ASCII and nothing else, so this has to fold the same
/// way: matching Unicode here would drop a tag the database still tells apart.
fn deduplicated(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();

    tags.into_iter().filter(|tag| seen.insert(tag.to_ascii_lowercase())).collect()
}

const DEFAULT: &str = include_str!("../landing-page.toml");

/// The six things a section can be
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Books,
    New,
    Authors,
    Tags,
    Series,
    Publishers,
}

impl Kind {
    /// The path this kind lives under, below `/{lib}`.
    pub fn feed(self) -> &'static str {
        match self {
            Kind::Books => "books",
            Kind::New => "new",
            Kind::Authors => "authors",
            Kind::Tags => "tags",
            Kind::Series => "series",
            Kind::Publishers => "publishers",
        }
    }

    /// What a section of this kind is called when the config does not say.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Books => "All Books",
            Kind::New => "Recently Added",
            Kind::Authors => "Authors",
            Kind::Tags => "Tags",
            Kind::Series => "Series",
            Kind::Publishers => "Publishers",
        }
    }

    /// Authors, tags, series and publishers each are a feed of links,
    /// so there is nothing there to filter and nothing to show as covers
    pub fn is_shelf(self) -> bool {
        matches!(self, Kind::Books | Kind::New)
    }
}

#[derive(Debug)]
pub struct Section {
    pub kind: Kind,
    pub title: String,
    pub id: Option<String>,     // only required with tags_any or tags_all
    pub tags_any: Vec<String>,  // limit to books with at least one of these tags
    pub tags_all: Vec<String>,  //                     all of these tags
    pub shown: usize,           // how many book-covers to show directly on the landing page
}

/// A section as it is written, before anything about it has been checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    #[serde(rename = "type")]
    kind: Kind,
    title: Option<String>,
    id: Option<String>,
    #[serde(default)]
    tags_any: Vec<String>,
    #[serde(default)]
    tags_all: Vec<String>,
    #[serde(default)]
    show: usize,
}

impl Section {
    fn of(kind: Kind) -> Section {
        Section {
            kind,
            title: kind.title().to_string(),
            id: None,
            tags_any: Vec::new(),
            tags_all: Vec::new(),
            shown: 0,
        }
    }

    // validity of a section is checked during startup
    fn checked(written: Written) -> Result<Section> {
        let named = written.title.unwrap_or_else(|| written.kind.title().to_string());
        let filtered = !written.tags_any.is_empty() || !written.tags_all.is_empty();

        if !written.tags_any.is_empty() && !written.tags_all.is_empty() {
            return Err(anyhow!(
                // todo: consider if this restriction is really necessary
                "section \"{}\": You can't use both tags_any and tags_all together: Choose one.",
                named
            ));
        }

        if !written.kind.is_shelf() && (filtered || written.show > 0) {
            return Err(anyhow!(
                "section \"{}\": type = \"{}\" is a list of links rather than of books, \
                 so it takes no tags_any, tags_all or show.",
                named,
                written.kind.feed()
            ));
        }

        if filtered && written.id.is_none() {
            return Err(anyhow!(
                "section \"{}\": a section that filters needs an id -- it is the address \
                 the section leads to, at /{{lib}}/shelves/{{id}}.",
                named
            ));
        }

        if let Some(id) = &written.id {
            if id.trim().is_empty() || id.contains('/') {
                return Err(anyhow!(
                    "section \"{}\": id = \"{}\" -- an id is one segment of a URL and can't be empty.",
                    named,
                    id
                ));
            }
        }

        Ok(Section {
            kind: written.kind,
            title: named,
            id: written.id,
            tags_any: deduplicated(written.tags_any),
            tags_all: deduplicated(written.tags_all),
            shown: written.show,
        })
    }

    /// `show = 0` and absence of `show`: render a link.
    pub fn inline(&self) -> bool {
        self.shown > 0
    }

    /// are we talking about some of the library or all of it?
    pub fn filtered(&self) -> bool {
        !self.tags_any.is_empty() || !self.tags_all.is_empty()
    }

    /// The tags a book has to carry, and whether it needs every one of them.
    pub fn tags(&self) -> (&[String], bool) {
        match self.tags_all.is_empty() {
            true => (&self.tags_any, false),
            false => (&self.tags_all, true),
        }
    }

    pub fn shelf(&self) -> Option<Shelf> {
        let (tags, all) = self.tags();

        match &self.id {
            Some(slug) if self.filtered() => Some(Shelf::Named(Named {
                slug: slug.clone(),
                title: self.title.clone(),
                tags: tags.to_vec(),
                all,
            })),
            _ => None,
        }
    }

    /// Where this section leads, below `/{lib}`.
    pub fn feed(&self) -> String {
        match self.filtered() {
            true => format!("shelves/{}", self.id.as_deref().unwrap_or_default()),
            false => self.kind.feed().to_string(),
        }
    }

    /// How many things this section leads to
    pub fn total(&self, counts: &Counts) -> Option<usize> {
        if self.filtered() {
            return None;
        }

        match self.kind {
            Kind::Books => Some(counts.books),
            Kind::New => None,
            Kind::Authors => Some(counts.authors),
            Kind::Tags => Some(counts.tags),
            Kind::Series => Some(counts.series),
            Kind::Publishers => Some(counts.publishers),
        }
    }

    /// The books the section puts directly landing page ("inline").
    /// newest first if the section is a `new` one.
    pub fn preview(&self, db: &Connection) -> rusqlite::Result<Vec<Book>> {
        let (tags, all) = self.tags();

        let mut books = match (self.kind, self.filtered()) {
            (Kind::New, true) => calibre::newest_tagged(db, tags, all)?,
            (Kind::New, false) => calibre::recently_added(db)?,
            (_, true) => calibre::books_tagged_page(db, tags, all, self.shown, 0)?,
            (_, false) => calibre::books_page(db, self.shown, 0)?,
        };
        books.truncate(self.shown);

        Ok(books)
    }

    /// Whether this section is offered at all
    pub fn visible(&self, db: &Connection, counts: &Counts) -> rusqlite::Result<bool> {
        if self.filtered() {
            let (tags, all) = self.tags();
            return calibre::any_books_tagged(db, tags, all);
        }

        Ok(self.kind.is_shelf() || self.total(counts).is_some_and(|total| total > 0))
    }

    pub fn description(&self, counts: &Counts) -> String {
        if self.filtered() {
            let (tags, all) = self.tags();
            let joined = tags.join(match all {
                true => " and ",
                false => " or ",
            });

            return match self.kind {
                Kind::New => format!("The newest books tagged {}", joined),
                _ => format!("Books tagged {}", joined),
            };
        }

        match self.kind {
            Kind::Books => format!("All {} books, by title", counts.books),
            Kind::New => "The newest additions to the library".to_string(),
            _ => format!("{} {}", self.total(counts).unwrap_or(0), self.title.to_lowercase()),
        }
    }
}

impl<'de> serde::Deserialize<'de> for Section {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let written = <Written as serde::Deserialize>::deserialize(deserializer)?;
        Section::checked(written).map_err(de::Error::custom)
    }
}

#[derive(Debug)]
pub struct Landing {
    sections: Vec<Section>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    #[serde(default)]
    section: Vec<Section>,
}

impl Landing {
    pub fn parse(source: &str) -> Result<Landing> {
        let page: Page = toml::from_str(source).map_err(|err| anyhow!(err))?;

        if page.section.is_empty() {
            return Err(anyhow!(
                "no [[section]] at all: a landing page says what a library offers, \
                 and this one offers nothing."
            ));
        }

        let mut ids = HashSet::new();
        for id in page.section.iter().filter_map(|section| section.id.as_deref()) {
            if !ids.insert(id) {
                return Err(anyhow!("id = \"{}\" is written twice: an id is one shelf's address.", id));
            }
        }

        Ok(Landing { sections: page.section })
    }

    pub fn read(path: &Path) -> Result<Landing> {
        let named = |err: &dyn fmt::Display| anyhow!("landing page '{}': {}", path.display(), err);

        let source = fs::read_to_string(path).map_err(|err| named(&err))?;

        Landing::parse(&source).map_err(|err| named(&err))
    }

    /// The section `/{lib}/shelves/{slug}`
    /// the same address is a 404 in a library that doesn't define it.
    pub fn shelf(&self, slug: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|section| section.filtered() && section.id.as_deref() == Some(slug))
    }

    pub fn visible(&self, db: &Connection, counts: &Counts) -> rusqlite::Result<Vec<&Section>> {
        static ALL_BOOKS: Lazy<Section> = Lazy::new(|| Section::of(Kind::Books));

        let mut shown = Vec::new();
        for section in &self.sections {
            if section.visible(db, counts)? {
                shown.push(section);
            }
        }

        Ok(match shown.is_empty() {
            true => vec![&ALL_BOOKS],
            false => shown,
        })
    }

    #[cfg(test)]
    fn kinds(&self) -> Vec<Kind> {
        self.sections.iter().map(|section| section.kind).collect()
    }
}

/// The page compiled into the binary
pub fn default_page() -> &'static Landing {
    static PAGE: Lazy<Landing> =
        Lazy::new(|| Landing::parse(DEFAULT).expect("the built-in landing page has to parse"));

    &PAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(books: usize, authors: usize, tags: usize, series: usize, publishers: usize) -> Counts {
        Counts { books, authors, tags, series, publishers }
    }

    /// The fixture library
    fn library() -> Connection {
        Connection::open("tests/calibre/metadata.db").expect("test library")
    }

    fn shown(page: &Landing, counts: &Counts) -> Vec<String> {
        page.visible(&library(), counts)
            .expect("a landing page")
            .iter()
            .map(|section| section.title.clone())
            .collect()
    }

    fn refusal(source: &str, why: &str) -> String {
        match Landing::parse(source) {
            Ok(_) => panic!("{}", why),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn the_built_in_page_is_the_root_orca_always_had() {
        let page = default_page();

        assert_eq!(
            page.kinds(),
            [Kind::Books, Kind::New, Kind::Authors, Kind::Tags, Kind::Series, Kind::Publishers]
        );

        assert_eq!(
            shown(page, &counts(918, 300, 12, 40, 8)),
            ["All Books", "Recently Added", "Authors", "Tags", "Series", "Publishers"]
        );
    }

    #[test]
    fn a_signpost_pointing_at_nothing_is_left_off_the_page() {
        // The books are still offered, empty library or not.
        assert_eq!(shown(default_page(), &counts(4, 0, 0, 0, 0)), ["All Books", "Recently Added"]);
    }

    #[test]
    fn a_page_where_everything_was_suppressed_still_leads_to_the_books() {
        let page = Landing::parse("[[section]]\ntype = \"tags\"\n").unwrap();

        assert_eq!(shown(&page, &counts(0, 0, 0, 0, 0)), ["All Books"]);
    }

    #[test]
    fn a_section_is_called_what_its_type_is_called_until_it_says_otherwise() {
        let page = Landing::parse(
            "[[section]]\ntype = \"tags\"\ntitle = \"Genres\"\n\n[[section]]\ntype = \"series\"\n",
        )
        .unwrap();

        let full = counts(10, 2, 3, 4, 5);
        let db = library();
        let shown = page.visible(&db, &full).unwrap();

        assert_eq!(shown[0].title, "Genres");
        assert_eq!(shown[1].title, "Series");

        assert_eq!(shown[0].description(&full), "3 genres");
        assert_eq!(shown[1].description(&full), "4 series");
    }

    #[test]
    fn renaming_a_section_leaves_the_feed_it_leads_to_where_it_was() {
        let page = Landing::parse("[[section]]\ntype = \"tags\"\ntitle = \"Genres\"\n").unwrap();

        let db = library();
        assert_eq!(page.visible(&db, &counts(1, 0, 7, 0, 0)).unwrap()[0].feed(), "tags");
    }

    #[test]
    fn a_type_orca_does_not_serve_offers_the_ones_it_does() {
        let err = refusal("[[section]]\ntype = \"shelves\"\n", "an unknown type must not parse");

        assert!(err.contains("books"), "{}", err);
        assert!(err.contains("publishers"), "{}", err);
    }

    #[test]
    fn a_misspelled_key_refuses_to_parse() {
        let err = refusal(
            "[[section]]\ntype = \"books\"\ntitel = \"All of it\"\n",
            "a misspelled key must not be ignored",
        );

        assert!(err.contains("unknown field `titel`"), "{}", err);
    }

    #[test]
    fn a_page_with_nothing_on_it_refuses_to_parse() {
        let err = refusal("# nothing here\n", "an empty landing page must not start a server");

        assert!(err.contains("no [[section]]"), "{}", err);
    }

    #[test]
    fn a_section_cannot_ask_for_any_tag_and_every_tag_at_once() {
        let err = refusal(
            "[[section]]\nid = \"x\"\ntype = \"books\"\ntags_any = [\"a\"]\ntags_all = [\"b\"]\n",
            "two filters on one section must not parse",
        );

        assert!(err.contains("tags_any") && err.contains("tags_all"), "{}", err);
    }

    // Authors, tags, series and publishers are feeds of links: there are no
    // books in them to filter, and nothing to show as covers.
    #[test]
    fn a_list_of_links_takes_none_of_the_keys_that_describe_books() {
        for key in ["tags_any = [\"a\"]", "tags_all = [\"a\"]", "show = 4"] {
            let source = format!("[[section]]\ntype = \"authors\"\n{}\n", key);
            let err = refusal(&source, "a category must not take a book key");

            assert!(err.contains("list of links"), "{}: {}", key, err);
        }
    }

    #[test]
    fn a_section_that_filters_needs_an_address_of_its_own() {
        let err = refusal(
            "[[section]]\ntype = \"books\"\ntags_any = [\"scifi\"]\n",
            "a filtered section without an id must not parse",
        );

        assert!(err.contains("needs an id"), "{}", err);
    }

    #[test]
    fn an_id_that_is_no_path_segment_refuses_to_parse() {
        for id in ["", " ", "science/fiction"] {
            let source = format!("[[section]]\ntype = \"books\"\nid = \"{}\"\n", id);
            let err = refusal(&source, "an id that is no segment must not parse");

            assert!(err.contains("one segment of a URL"), "{:?}: {}", id, err);
        }
    }

    #[test]
    fn two_sections_cannot_share_one_address() {
        let err = refusal(
            "[[section]]\ntype = \"books\"\nid = \"same\"\n\n[[section]]\ntype = \"new\"\nid = \"same\"\n",
            "a duplicate id must not parse",
        );

        assert!(err.contains("written twice"), "{}", err);
    }

    #[test]
    fn a_section_shows_as_many_books_as_it_asked_for_and_no_number_is_refused() {
        for (show, shown) in [("", 0), ("show = 0\n", 0), ("show = 1\n", 1), ("show = 40\n", 40)] {
            let page = one(&format!("[[section]]\ntype = \"new\"\n{}", show));
            let section = &page.visible(&library(), &counts(7, 8, 7, 1, 3)).unwrap()[0];

            assert_eq!(section.shown, shown, "show = {:?}", show);
            assert_eq!(section.inline(), shown > 0, "show = {:?}", show);
        }
    }

    fn one(source: &str) -> Landing {
        Landing::parse(source).expect("a landing page")
    }

    // A filtered section leads to an address of its own, and the slug is not derived from the title.
    #[test]
    fn a_filtered_section_lives_under_the_slug_it_was_given() {
        let page = one(
            "[[section]]\nid = \"scifi\"\ntitle = \"Scifi & Fantasy\"\ntype = \"books\"\n\
             tags_any = [\"science fiction\", \"fantasy\"]\n",
        );
        let section = page.shelf("scifi").expect("a shelf under its slug");

        assert_eq!(section.feed(), "shelves/scifi");
        assert_eq!(section.tags(), (["science fiction".to_string(), "fantasy".to_string()].as_slice(), false));

        let shelf = section.shelf().expect("a filtered section is a shelf");
        assert_eq!(shelf.path(), "shelves/scifi");
        assert_eq!(shelf.feed(), "shelves");
        assert_eq!(shelf.name(&library()).unwrap(), "Scifi & Fantasy");
    }

    #[test]
    fn tags_all_asks_for_every_one_of_them() {
        let page = one(
            "[[section]]\nid = \"both\"\ntype = \"books\"\ntags_all = [\"magazine\", \"politics\"]\n",
        );
        let section = page.shelf("both").unwrap();

        assert!(section.tags().1, "tags_all wants every one of them");
    }

    // Only a filtered section has an address of its own
    #[test]
    fn a_section_that_filters_nothing_is_no_named_shelf() {
        let page = one("[[section]]\ntype = \"books\"\n\n[[section]]\ntype = \"tags\"\n");

        assert!(page.shelf("books").is_none());
        assert_eq!(page.visible(&library(), &counts(7, 8, 7, 1, 3)).unwrap()[0].feed(), "books");
    }

    #[test]
    fn a_tag_written_twice_is_asked_for_once() {
        let page = one(
            "[[section]]\nid = \"sf\"\ntype = \"books\"\n\
             tags_all = [\"Science Fiction\", \"science fiction\"]\n",
        );

        assert_eq!(page.shelf("sf").unwrap().tags().0, ["Science Fiction".to_string()]);
    }

    // The blurb of a filtered section cannot quote a number. It says which books it holds instead.
    #[test]
    fn a_filtered_section_describes_itself_by_its_tags() {
        let full = counts(7, 8, 7, 1, 3);

        let any = one("[[section]]\nid = \"sf\"\ntype = \"books\"\ntags_any = [\"scifi\", \"fantasy\"]\n");
        assert_eq!(any.shelf("sf").unwrap().description(&full), "Books tagged scifi or fantasy");

        let all = one("[[section]]\nid = \"x\"\ntype = \"books\"\ntags_all = [\"scifi\", \"horror\"]\n");
        assert_eq!(all.shelf("x").unwrap().description(&full), "Books tagged scifi and horror");

        let new = one("[[section]]\nid = \"n\"\ntype = \"new\"\ntags_any = [\"magazine\"]\n");
        assert_eq!(new.shelf("n").unwrap().description(&full), "The newest books tagged magazine");
    }

    #[test]
    fn a_filtered_section_is_offered_without_being_counted() {
        let page = one("[[section]]\nid = \"sf\"\ntype = \"books\"\ntags_any = [\"science fiction\"]\n");
        let section = page.shelf("sf").unwrap();
        let db = library();

        assert_eq!(section.total(&counts(7, 8, 7, 1, 3)), None);
        assert!(section.visible(&db, &counts(7, 8, 7, 1, 3)).unwrap());
    }

    fn previewed(source: &str) -> Vec<String> {
        let page = one(source);
        let db = library();
        let sections = page.visible(&db, &counts(7, 8, 7, 1, 3)).unwrap();

        sections[0].preview(&db).expect("the books of a section").into_iter().map(|book| book.title).collect()
    }

    // A section shows the first few of its shelf, and asks for no more of the library
    #[test]
    fn an_inline_section_shows_the_start_of_its_shelf() {
        assert_eq!(previewed("[[section]]\ntype = \"books\"\nshow = 2\n").len(), 2);

        // One book is a section like any other, not a row too short to draw.
        assert_eq!(previewed("[[section]]\ntype = \"books\"\nshow = 1\n").len(), 1);

        // The whole shelf, when it is shorter than the number asked for.
        assert_eq!(previewed("[[section]]\ntype = \"books\"\nshow = 12\n").len(), 7);
    }

    // `new` is capped by the library rather than by the section
    #[test]
    fn an_inline_new_section_is_cut_to_the_number_it_asked_for() {
        assert_eq!(previewed("[[section]]\ntype = \"new\"\nshow = 3\n").len(), 3);
    }

    // first filtered, then capped: Four is the fewest that can assert the difference:
    // the philosophy book is the fourth of the library and it's not on the science fiction shelf.
    #[test]
    fn an_inline_filtered_section_shows_the_start_of_what_it_matched() {
        let scifi = previewed(
            "[[section]]\nid = \"sf\"\ntype = \"books\"\nshow = 4\n\
             tags_any = [\"science fiction\", \"fantasy\"]\n",
        );

        assert_eq!(scifi.len(), 4);
        assert!(!scifi.iter().any(|title| title.starts_with("Kritik")), "{:?}", scifi);
        assert!(scifi.contains(&"Vampires of Space".to_string()), "{:?}", scifi);
    }

    #[test]
    fn a_filtered_section_no_book_matches_is_left_off_the_page() {
        let page = one(
            "[[section]]\ntype = \"books\"\n\n\
             [[section]]\nid = \"cookery\"\ntitle = \"Cookery\"\ntype = \"books\"\n\
             tags_any = [\"cooking\"]\n",
        );

        assert_eq!(shown(&page, &counts(7, 8, 7, 1, 3)), ["All Books"]);
        // the shelf is there but it answers with an empty feed.
        assert!(page.shelf("cookery").is_some());
    }
}
