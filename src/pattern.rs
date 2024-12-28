extern crate serde;
extern crate regex;
extern crate toml;
extern crate anyhow;

use regex::Regex;
use serde::{Serialize, Serializer};
use serde::de::{self, Deserialize, Deserializer};
use anyhow::{Result, anyhow};

#[derive(Debug)]
pub struct Pattern {
    pub pattern: String,
    pub regex: Regex,
}

impl Pattern {
    pub fn new(pattern: &str) -> Result<Self> {
        if pattern.matches("**").count() > 1 {
            return Err(anyhow!("Pattern contains more than one '**'"));
        }

        let regex_pattern = pattern
            .replace("/", "\\/")
            .replace(".", "\\.")
            .replace("**", "_DOUBLESTAR_") // Replace '**' with a unique string
            .replace("*", "[^/]*")
            .replace("_DOUBLESTAR_", ".*") // ...to avoid clashes with the single '*'
            .replace("?", "[^/]")
            .replace("\\/\\/", "\\/"); // x/**/y -> x/y not x//y

        let regex_pattern = format!("^{}$", regex_pattern);
        let regex = Regex::new(&regex_pattern)?;

        Ok(Self {
            pattern: pattern.to_string(),
            regex,
        })
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Pattern::new(&s).map_err(de::Error::custom)
    }
}

impl Serialize for Pattern {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_derive::{Deserialize, Serialize};
    use toml;

    #[derive(Deserialize, Serialize)]
    struct TestWrap {
        pattern: Pattern,
    }

    #[test]
    fn pattern_new() {
        let pattern_str = "/baz/*";
        let pattern = Pattern::new(pattern_str).unwrap();
        assert_eq!(pattern.pattern, pattern_str);
        assert!(pattern.regex.is_match("/baz/anything"));
        assert!(!pattern.regex.is_match("/foo/bar"));
    }

    #[test]
    fn pattern_deserialize() {
        let toml_data = r#"pattern = "/baz/*""#;
        let wrap: TestWrap = toml::from_str(&toml_data).unwrap();
        assert_eq!(wrap.pattern.pattern, "/baz/*");
        assert!(wrap.pattern.regex.is_match("/baz/anything"));
        assert!(!wrap.pattern.regex.is_match("/foo/bar"));
    }

    #[test]
    fn pattern_serialize() {
        let pattern = Pattern::new("/baz/*").unwrap();
        let wrap = TestWrap { pattern };
        let toml_data = toml::to_string(&wrap).unwrap();
        let expected = r#"pattern = "/baz/*""#;
        assert_eq!(toml_data.trim(), expected);
    }

    #[test]
    fn pattern_roundtrip() {
        let toml_data = r#"pattern = "/baz/*""#;
        let wrap: TestWrap = toml::from_str(&toml_data).unwrap();
        let serialized = toml::to_string(&wrap).unwrap();
        assert_eq!(toml_data.trim(), serialized.trim());
    }

    #[test]
    fn pattern_regex_conversion() {
        let patterns = [
            ("/", r"^\/$"),
            ("/bar", r"^\/bar$"),
            ("/baz/*", r"^\/baz\/[^/]*$"),
            ("/baz/**", r"^\/baz\/.*$"),
            ("/foo/*/bar", r"^\/foo\/[^/]*\/bar$"),
            ("/foo/?/bar", r"^\/foo\/[^/]\/bar$"),
        ];

        for (pattern_str, expected_regex) in &patterns {
            let pattern = Pattern::new(pattern_str).unwrap();
            assert_eq!(pattern.regex.as_str(), *expected_regex,
                "Failed for pattern: '{}'", pattern_str);
        }
    }

    #[test]
    fn pattern_multiple_double_asterisks() {
        let pattern_str = "/foo/**/bar/**";
        let pattern = Pattern::new(pattern_str);
        assert!(pattern.is_err());
    }
}
