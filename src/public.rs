//! Which requests Orca serves without credentials.
//!
//! A `public` entry names a **library**, not a URL. 
//! e.g. `/fantasy` is a library in both feed formats, including book covers and downloads it points to

use anyhow::{anyhow, Result};
use serde::de::{self, Deserialize, Deserializer};
use serde::{Serialize, Serializer};

/// The feed formats
pub const VERSIONS: [&str; 2] = ["v1", "v2"];

/// e.g. `/{lib}/books` redirects to `/v1/{lib}/books`
const BARE: &str = "v1";

/// these two are agnostic to the feed format that linked to them
const FILES: [&str; 2] = ["cover", "file"];

/// `/health` and `/auth` never get here: they're public
#[derive(Debug, PartialEq, Eq)]
pub enum Resource<'a> {
    /// `/`, `/v1`, `/v2` -- the list of libraries, in whichever format.
    Catalog,
    /// A feed inside one library.
    Feed { version: &'a str, name: &'a str },
    /// A cover or a download.
    File { name: &'a str },
}

impl<'a> Resource<'a> {
    pub fn of(path: &'a str) -> Self {
        let mut segments = path.split('/').filter(|segment| !segment.is_empty());

        let (version, name) = match segments.next() {
            None => return Resource::Catalog,
            Some(version) if VERSIONS.contains(&version) => match segments.next() {
                None => return Resource::Catalog,
                Some(name) => (version, name),
            },
            Some(name) => (BARE, name),
        };

        match segments.next() {
            Some(segment) if FILES.contains(&segment) => Resource::File { name },
            _ => Resource::Feed { version, name },
        }
    }
}

/// What one `public` entry opens.
#[derive(Debug, PartialEq, Eq)]
enum Scope {
    /// `/` -- the list of libraries, in both formats.
    Catalog,
    /// `/{lib}`, or `/v1/{lib}` / `/v2/{lib}` for one format of it.
    Library { version: Option<String>, name: String },
    /// A route that needs no credentials (`/health` is the only one)
    Nothing,
}

/// One entry of `[authentication] public`.
#[derive(Debug)]
pub struct Rule {
    source: String,
    scope: Scope,
}

impl Rule {
    pub fn new(source: &str) -> Result<Self> {
        Ok(Rule { scope: scope_of(source)?, source: source.to_string() })
    }
    /// The library this rule refers to, checked against the
    /// libraries actually configured at startup
    pub fn library(&self) -> Option<&str> {
        match &self.scope {
            Scope::Library { name, .. } => Some(name),
            _ => None,
        }
    }

    fn opens(&self, resource: &Resource) -> bool {
        match (&self.scope, resource) {
            (Scope::Catalog, Resource::Catalog) => true,

            // A rule naming a format opens that format's feeds ...
            (Scope::Library { version, name }, Resource::Feed { version: asked, name: lib }) => {
                name == lib && version.as_deref().is_none_or(|version| version == *asked)
            }

            // ... and the files either format points at
            (Scope::Library { name, .. }, Resource::File { name: lib }) => name == lib,

            _ => false,
        }
    }
}

fn scope_of(source: &str) -> Result<Scope> {
    if source.trim_start_matches('/') == "**" {
        return Err(anyhow!(
            "public = \"{}\": every route public is not a whitelist. \
             Leave out the whole [authentication] section to serve without authentication.",
            source
        ));
    }

    // deprecated: `/{lib}/**` is how a library was spelled before
    let named = source.strip_suffix("/**").unwrap_or(source);
    let segments: Vec<&str> = named.split('/').filter(|segment| !segment.is_empty()).collect();

    if segments.iter().any(|segment| segment.contains('*') || segment.contains('?')) {
        return Err(anyhow!(
            "public = \"{}\": wildcards are gone. Name each public library: \
             \"/fantasy\", or \"/v2/fantasy\" for one format of it.",
            source
        ));
    }

    match segments[..] {
        [] => Ok(Scope::Catalog),
        ["health"] => Ok(Scope::Nothing),
        [version] if VERSIONS.contains(&version) => Err(anyhow!(
            "public = \"{}\": the catalog is \"/\", in both formats at once.",
            source
        )),
        [name] => Ok(Scope::Library { version: None, name: name.to_string() }),
        [version, name] if VERSIONS.contains(&version) => Ok(Scope::Library {
            version: Some(version.to_string()),
            name: name.to_string(),
        }),
        _ => Err(anyhow!(
            "public = \"{}\": an entry names a library -- \"/fantasy\", or \
             \"/v1/fantasy\" / \"/v2/fantasy\" for one format of it. \
             Nothing smaller than a library can be made public.",
            source
        )),
    }
}

/// Whether a request carrying no credentials may be served.
pub fn allows(rules: &[Rule], path: &str) -> bool {
    let resource = Resource::of(path);
    rules.iter().any(|rule| rule.opens(&resource))
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let source = String::deserialize(deserializer)?;
        Rule::new(&source).map_err(de::Error::custom)
    }
}

impl Serialize for Rule {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(sources: &[&str]) -> Vec<Rule> {
        sources.iter().map(|source| Rule::new(source).unwrap()).collect()
    }

    #[test]
    fn a_path_says_which_library_it_belongs_to() {
        assert_eq!(Resource::of("/"), Resource::Catalog);
        assert_eq!(Resource::of("/v1"), Resource::Catalog);
        assert_eq!(Resource::of("/v2"), Resource::Catalog);
        assert_eq!(Resource::of("/v2/foo"), Resource::Feed { version: "v2", name: "foo" });
        assert_eq!(
            Resource::of("/v2/foo/authors/5"),
            Resource::Feed { version: "v2", name: "foo" }
        );
        assert_eq!(Resource::of("/foo/cover/5"), Resource::File { name: "foo" });
        assert_eq!(Resource::of("/foo/file/4/epub"), Resource::File { name: "foo" });
    }

    // The bare spelling is a redirect to `/v1`
    #[test]
    fn a_feed_named_without_a_format_is_the_atom_one() {
        assert_eq!(Resource::of("/foo"), Resource::Feed { version: "v1", name: "foo" });
        assert_eq!(Resource::of("/foo/books"), Resource::Feed { version: "v1", name: "foo" });
    }

    #[test]
    fn a_library_is_public_in_both_formats_with_its_files() {
        let public = rules(&["/foo"]);

        for path in ["/foo", "/foo/books", "/v1/foo", "/v1/foo/books", "/v2/foo",
                     "/v2/foo/books", "/foo/cover/5", "/foo/file/4/epub"] {
            assert!(allows(&public, path), "{} should need no credentials", path);
        }

        for path in ["/", "/v1", "/v2"] {
            assert!(!allows(&public, path), "{} is the catalog, not the library", path);
        }
    }

    #[test]
    fn naming_a_format_opens_that_format_and_the_files_it_points_at() {
        let public = rules(&["/v2/foo"]);

        for path in ["/v2/foo", "/v2/foo/books", "/foo/cover/5", "/foo/file/4/epub"] {
            assert!(allows(&public, path), "{} should need no credentials", path);
        }

        // Including the bare alias, which leads to `/v1`.
        for path in ["/v1/foo", "/v1/foo/books", "/foo", "/foo/books"] {
            assert!(!allows(&public, path), "{} is the format that was not named", path);
        }
    }

    #[test]
    fn one_library_is_never_another_that_starts_the_same_way() {
        for public in [rules(&["/foo"]), rules(&["/v2/foo"]), rules(&["/foo/**"])] {
            for path in ["/v1/foobar", "/v2/foobar", "/foobar", "/foobar/books",
                         "/foobar/cover/5", "/foobar/file/4/epub"] {
                assert!(!allows(&public, path), "{} belongs to another library", path);
            }
        }
    }

    #[test]
    fn the_catalog_is_named_on_its_own() {
        let public = rules(&["/"]);

        for path in ["/", "/v1", "/v2"] {
            assert!(allows(&public, path), "{} is the catalog", path);
        }
        for path in ["/foo", "/v2/foo", "/foo/cover/5"] {
            assert!(!allows(&public, path), "{} is a library, not the catalog", path);
        }
    }

    // deprecated: but existing configs should continue to work
    #[test]
    fn the_old_subtree_spelling_still_reads() {
        assert_eq!(Rule::new("/foo/**").unwrap().scope, Rule::new("/foo").unwrap().scope);
        assert_eq!(Rule::new("/v2/foo/**").unwrap().scope, Rule::new("/v2/foo").unwrap().scope);
    }

    #[test]
    fn health_is_accepted_and_opens_nothing() {
        let public = rules(&["/health"]);
        assert!(!allows(&public, "/foo"));
        assert!(!allows(&public, "/"));
    }

    #[test]
    fn an_entry_that_names_no_library_is_refused() {
        for source in ["/**", "**", "/v2/foo/tags", "/*/cover/*", "/foo/cover/5", "/v1"] {
            let refusal = Rule::new(source);
            assert!(refusal.is_err(), "{} should not parse", source);
        }
    }

    #[test]
    fn a_refusal_quotes_the_entry_and_says_what_to_write() {
        let err = Rule::new("/**").unwrap_err().to_string();
        assert!(err.contains("/**"), "{}", err);
        assert!(err.contains("[authentication]"), "{}", err);

        let err = Rule::new("/v2/foo/tags").unwrap_err().to_string();
        assert!(err.contains("/v2/foo/tags"), "{}", err);
        assert!(err.contains("names a library"), "{}", err);
    }

    #[test]
    fn an_entry_names_the_library_it_opens() {
        assert_eq!(Rule::new("/foo").unwrap().library(), Some("foo"));
        assert_eq!(Rule::new("/v2/foo").unwrap().library(), Some("foo"));
        assert_eq!(Rule::new("/").unwrap().library(), None);
        assert_eq!(Rule::new("/health").unwrap().library(), None);
    }
}
