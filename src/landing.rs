
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

use crate::calibre::Counts;

const DEFAULT: &str = include_str!("../landing-page.toml");

/// How many books an inline section shows unless it says otherwise
const BOOKS_SHOWN: usize = 12;

const FEWEST_SHOWN: usize = 2;

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
    pub id: Option<String>,
    pub tags_any: Vec<String>, // At least one of these tags.
    pub tags_all: Vec<String>, // All of these tags.
    pub inline: bool, // show Covers directly on the landing page (not just a link)
    pub shown: usize, // in case we show covers: how many?
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
    inline: bool,
    count: Option<usize>,
}

impl Section {
    fn of(kind: Kind) -> Section {
        Section {
            kind,
            title: kind.title().to_string(),
            id: None,
            tags_any: Vec::new(),
            tags_all: Vec::new(),
            inline: false,
            shown: BOOKS_SHOWN,
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

        if !written.kind.is_shelf() && (filtered || written.inline || written.count.is_some()) {
            return Err(anyhow!(
                "section \"{}\": type = \"{}\" is a list of links rather than of books, \
                 so it takes no tags_any, tags_all, inline or count.",
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

        if let Some(count) = written.count {
            if count < FEWEST_SHOWN {
                return Err(anyhow!(
                    "section \"{}\": count = {} -- a row of one book is drawn by no client at all. \
                     The fewest worth asking for is {}.",
                    named,
                    count,
                    FEWEST_SHOWN
                ));
            }
        }

        // todo: Until named shelves exist there is nowhere for a filtered section to lead
        if filtered {
            return Err(anyhow!(
                "section \"{}\": tags_any and tags_all are not served yet -- the named shelf \
                 they would lead to does not exist in this version.",
                named
            ));
        }

        Ok(Section {
            kind: written.kind,
            title: named,
            id: written.id,
            tags_any: written.tags_any,
            tags_all: written.tags_all,
            inline: written.inline,
            shown: written.count.unwrap_or(BOOKS_SHOWN),
        })
    }

    /// Where this section leads, below `/{lib}`.
    pub fn feed(&self) -> &str {
        self.kind.feed()
    }

    /// How many things this section leads to
    pub fn total(&self, counts: &Counts) -> Option<usize> {
        match self.kind {
            Kind::Books => Some(counts.books),
            Kind::New => None,
            Kind::Authors => Some(counts.authors),
            Kind::Tags => Some(counts.tags),
            Kind::Series => Some(counts.series),
            Kind::Publishers => Some(counts.publishers),
        }
    }

    pub fn visible(&self, counts: &Counts) -> bool {
        self.kind.is_shelf() || self.total(counts).is_some_and(|total| total > 0)
    }

    pub fn description(&self, counts: &Counts) -> String {
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

    pub fn visible(&self, counts: &Counts) -> Vec<&Section> {
        static ALL_BOOKS: Lazy<Section> = Lazy::new(|| Section::of(Kind::Books));

        let shown: Vec<&Section> =
            self.sections.iter().filter(|section| section.visible(counts)).collect();

        match shown.is_empty() {
            true => vec![&ALL_BOOKS],
            false => shown,
        }
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

        let full = counts(918, 300, 12, 40, 8);
        let titles: Vec<&str> =
            page.visible(&full).iter().map(|section| section.title.as_str()).collect();
        assert_eq!(titles, ["All Books", "Recently Added", "Authors", "Tags", "Series", "Publishers"]);
    }

    #[test]
    fn a_signpost_pointing_at_nothing_is_left_off_the_page() {
        let bare = counts(4, 0, 0, 0, 0);
        let titles: Vec<&str> = default_page()
            .visible(&bare)
            .iter()
            .map(|section| section.title.as_str())
            .collect();

        // The books are still offered, empty library or not.
        assert_eq!(titles, ["All Books", "Recently Added"]);
    }

    #[test]
    fn a_page_where_everything_was_suppressed_still_leads_to_the_books() {
        let page = Landing::parse("[[section]]\ntype = \"tags\"\n").unwrap();
        let empty = counts(0, 0, 0, 0, 0);

        let shown = page.visible(&empty);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].kind, Kind::Books);
        assert_eq!(shown[0].title, "All Books");
    }

    #[test]
    fn a_section_is_called_what_its_type_is_called_until_it_says_otherwise() {
        let page = Landing::parse(
            "[[section]]\ntype = \"tags\"\ntitle = \"Genres\"\n\n[[section]]\ntype = \"series\"\n",
        )
        .unwrap();

        let full = counts(10, 2, 3, 4, 5);
        let shown = page.visible(&full);

        assert_eq!(shown[0].title, "Genres");
        assert_eq!(shown[1].title, "Series");

        assert_eq!(shown[0].description(&full), "3 genres");
        assert_eq!(shown[1].description(&full), "4 series");
    }

    #[test]
    fn renaming_a_section_leaves_the_feed_it_leads_to_where_it_was() {
        let page = Landing::parse("[[section]]\ntype = \"tags\"\ntitle = \"Genres\"\n").unwrap();

        assert_eq!(page.visible(&counts(1, 0, 7, 0, 0))[0].feed(), "tags");
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
        for key in ["tags_any = [\"a\"]", "tags_all = [\"a\"]", "inline = true", "count = 4"] {
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

    // `count = 1` renders nothing
    #[test]
    fn a_row_of_one_book_is_refused_rather_than_drawn_as_nothing() {
        for count in [0, 1] {
            let source = format!("[[section]]\ntype = \"new\"\ninline = true\ncount = {}\n", count);
            let err = refusal(&source, "a count below two must not parse");

            assert!(err.contains("no client at all"), "{}: {}", count, err);
        }

        assert!(Landing::parse("[[section]]\ntype = \"new\"\ninline = true\ncount = 2\n").is_ok());
    }

    // todo: Named shelves. Until they are there, a filtered section would lead to a 404.
    #[test]
    fn a_filter_is_refused_while_there_is_nowhere_for_it_to_lead() {
        let err = refusal(
            "[[section]]\nid = \"scifi\"\ntype = \"books\"\ntags_any = [\"science fiction\"]\n",
            "a filter must not parse before the shelf it points at exists",
        );

        assert!(err.contains("not served yet"), "{}", err);
    }
}
