
use dirs::home_dir;
use std::{env, fmt, fs, collections::HashMap};
use serde_derive::{Deserialize, Serialize};
use crate::public::Rule;

use once_cell::sync::Lazy;
use anyhow::{Context, Error, Result, anyhow};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub server: Server,
    /// authentication means: everything is public
    #[serde(default)]
    pub authentication: Option<Authentication>,
    pub calibre: Calibre,
    #[serde(default)]
    pub catalog: Catalog,
}

impl Config {
    pub fn author(&self, lib: Option<&str>) -> &str {
        lib.and_then(|lib| self.calibre.libraries.get(lib))
            .and_then(|lib| lib.author.as_deref())
            .unwrap_or(&self.catalog.author)
    }

    /// libraries that are exempt from requiring credentials.
    pub fn public(&self) -> &[Rule] {
        match &self.authentication {
            Some(auth) => &auth.public,
            None => &[],
        }
    }
}

/// A scheme is written lowercase (RFC 3986)
/// `Http` is deprecated now
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum Protocol {
    #[serde(alias = "Http")]
    Http,
    #[serde(alias = "Https")]
    Https {
        cert: String,
        key: String,
    },
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Server {
    pub ip: String,
    pub port: u16,
    /// Externally visible base URL, e.g. "https://orca.example.com". Only needed
    /// when the reverse proxy in front of Orca does not set X-Forwarded-Proto /
    /// X-Forwarded-Host; otherwise feeds derive their own URL from the request.
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(flatten)]
    pub protocol: Protocol,
}

/// Either may be empty:
/// logins are present, but nothing is public -> authentication is obligatory for everything
/// no logins but some libraries are public -> serve only the public libraries.
/// but: no logins AND no public libraries -> error: there's nothing to serve
#[derive(Serialize, Deserialize, Default)]
pub struct Authentication {
    #[serde(default)]
    pub login: HashMap<String, String>,
    #[serde(default)]
    pub public: Vec<Rule>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Calibre {
    pub libraries: HashMap<String, Library>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Library {
    pub path: String,
    #[serde(default)]
    pub author: Option<String>,
}

/// the name of the catalog / title of its root feed
pub const CATALOG: &str = "ORCA";

/// How the catalog presents itself, as opposed to where its books live.
#[derive(Serialize, Deserialize, Clone)]
pub struct Catalog {
    #[serde(default = "orca")]
    pub author: String,
}

fn orca() -> String {
    "orca".to_string()
}

impl Default for Catalog {
    fn default() -> Self {
        Catalog { author: orca() }
    }
}

#[derive(Debug)]
struct PathError {
    path: String,
    error: Error,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} : {}", self.path, self.error)
    }
}

impl std::error::Error for PathError {}

fn find_config(paths: Vec<Option<String>>) -> Result<Config, Error> {
    let paths: Vec<String> = paths.into_iter().flatten().collect();
    let mut errors = Vec::new();

    for path in &paths {
        match read_config(path) {
            Ok(config) => {
                println!("Config loaded from: {}", path);
                return Ok(config);
            }
            // A file that is there and does not parse is the config the operator meant to use.
            Err(err) if valid_file(path) => {
                return Err(anyhow!(PathError { path: path.clone(), error: err }));
            }
            Err(err) => {
                errors.push(PathError { path: path.clone(), error: err });
            }
        }
    }

    let error_messages: String = errors.iter()
        .map(|e| format!("\t{} : {}", e.path, e.error))
        .collect::<Vec<String>>()
        .join("\n");

    Err(anyhow!(
        "No valid config file found:\n{}",
        error_messages
    ))
}

fn valid_file(config_file: &str) -> bool {
    fs::metadata(config_file).is_ok_and(|metadata| metadata.is_file())
}

pub fn read_config(config_file: &str) -> Result<Config, Error> {
    if !valid_file(config_file) {
        return Err(anyhow!("file not found"));
    }

    let contents = fs::read_to_string(config_file)
        .context("failed to read config file")?;
    let config = toml::from_str(&contents)
        .map_err(|err| anyhow!(err))?;
    Ok(config)
}

static CONFIG: Lazy<Config> = Lazy::new(|| {

    let conf_from_env: Option<String> = env::var("ORCA_CONFIG").ok();

    let local_conf1: Option<String> = home_dir().and_then(|path_buf| {
        path_buf.to_str().map(|s| format!("{}/.config/orca.toml", s.to_owned()))
    });

    let local_conf2: Option<String> = home_dir().and_then(|path_buf| {
        path_buf.to_str().map(|s| format!("{}/.config/orca/config.toml", s.to_owned()))
    });

    let configs = vec![conf_from_env, local_conf1, local_conf2];

    match find_config(configs) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Could not load config: {}", e);
            std::process::exit(1);
        }
    }
});

pub fn get() -> &'static Config {
    &CONFIG
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_valid_config_path() {
        let mut tmp_file = NamedTempFile::new().unwrap();
        let valid_toml = r#"
        [server]
        ip = "127.0.0.1"
        port = 8080
        protocol = "http"

        [authentication.login]
        alice = "...passwordhash..."

        [calibre]
        libraries = {}
        "#;
        write!(tmp_file, "{}", valid_toml).unwrap();

        let config = read_config(tmp_file.path().to_str().unwrap());
        assert!(config.is_ok());
        assert_eq!(config.unwrap().server.ip, "127.0.0.1");
    }

    #[test]
    fn test_missing_config_path() {
        let path = "/nonexistent/path/to/config.toml";

        let config = read_config(path);
        assert!(config.is_err());
        if let Err(e) = config {
            assert!(e.to_string().contains("file not found"));
        }
    }

    #[test]
    fn test_find_config_all_invalid_paths() {
        let path1 = "/nonexistent/path/to/config1.toml";
        let path2 = "/nonexistent/path/to/config2.toml";

        let config = find_config(vec![Some(path1.to_string()), Some(path2.to_string())]);
        assert!(config.is_err());
        if let Err(e) = config {
            assert!(e.to_string().contains("No valid config file found"));
        }
    }

    #[test]
    fn test_invalid_toml_syntax() {
        let mut tmp_file = NamedTempFile::new().unwrap();
        let invalid_toml = r#"
        [server
        ip = "127.0.0.1"
        port = 8080
        "#;
        write!(tmp_file, "{}", invalid_toml).unwrap();

        let config = read_config(tmp_file.path().to_str().unwrap());
        assert!(config.is_err());
        if let Err(e) = config {
            assert!(e.to_string().contains("TOML parse error"));
        }
    }

    #[test]
    fn test_invalid_toml_missing_field() {
        let mut tmp_file = NamedTempFile::new().unwrap();
        let invalid_toml = r#"
        [server]
        ip = "127.0.0.1"
        "#;
        write!(tmp_file, "{}", invalid_toml).unwrap();

        let config = read_config(tmp_file.path().to_str().unwrap());
        assert!(config.is_err());
        if let Err(e) = config {
            assert!(e.to_string().contains("missing field"));
        }
    }

    #[test]
    fn test_valid_file_nonexistent() {
        let path = "/nonexistent_path/to_config.toml";
        assert!(!valid_file(path));
    }

    #[test]
    fn test_protocol_https() {
        let valid_toml = r#"
        [server]
        ip = "127.0.0.1"
        port = 8080
        protocol = "https"
        cert = "/path/to/cert.pem"
        key = "/path/to/key.pem"

        [authentication.login]
        alice = "...passwordhash..."

        [calibre]
        libraries = {}
        "#;
        let mut tmp_file = NamedTempFile::new().unwrap();
        write!(tmp_file, "{}", valid_toml).unwrap();
        let config = read_config(tmp_file.path().to_str().unwrap());
        assert!(config.is_ok());
        if let Protocol::Https { cert, key } = &config.unwrap().server.protocol {
            assert_eq!(cert, "/path/to/cert.pem");
            assert_eq!(key, "/path/to/key.pem");
        } else {
            panic!("Expected Https protocol");
        }
    }

    fn config_file(contents: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{}", contents).unwrap();
        file
    }

    fn path_of(file: &NamedTempFile) -> Option<String> {
        Some(file.path().to_str().unwrap().to_string())
    }

    /// `unwrap_err` is unavailable here: it wants `Config: Debug`, which would
    /// drag a Debug bound through every type a config holds for no benefit.
    fn refusal(file: &NamedTempFile, why: &str) -> String {
        match read_config(file.path().to_str().unwrap()) {
            Ok(_) => panic!("{}", why),
            Err(err) => err.to_string(),
        }
    }

    const VALID: &str = r#"
        [server]
        ip = "127.0.0.1"
        port = 8080
        protocol = "http"

        [authentication.login]
        alice = "...passwordhash..."

        [calibre.libraries.library]
        path = "/mnt/library"
        "#;

    #[test]
    fn a_config_that_does_not_parse_stops_the_search() {
        // 0.1 wrote a library as a path; 0.2 wants a table.
        let broken = config_file(&VALID.replace(
            "[calibre.libraries.library]\n        path =",
            "[calibre.libraries]\n        library =",
        ));
        let fallback = config_file(VALID);

        let err = match find_config(vec![path_of(&broken), path_of(&fallback)]) {
            Ok(_) => panic!("a malformed config must not be skipped"),
            Err(err) => err.to_string(),
        };

        assert!(err.contains("expected struct Library"), "{}", err);
        assert!(err.contains(broken.path().to_str().unwrap()), "{}", err);
    }

    // The docs say lowercase; `Http` is deprecated, but still works for now
    #[test]
    fn a_protocol_is_read_in_the_spelling_its_config_was_written_in() {
        let tls = "\n        cert = \"cert.pem\"\n        key = \"key.pem\"";

        for (spelling, keys) in [
            ("http", ""), ("Http", ""),
            ("https", tls), ("Https", tls),
        ] {
            let named = config_file(&VALID.replacen("\"http\"", &format!("\"{}\"{}", spelling, keys), 1));

            if let Err(err) = read_config(named.path().to_str().unwrap()) {
                panic!("protocol = \"{}\" should parse: {}", spelling, err);
            }
        }
    }

    #[test]
    fn a_protocol_that_is_no_scheme_offers_the_canonical_spellings() {
        let typo = config_file(&VALID.replacen("\"http\"", "\"htpp\"", 1));

        let err = refusal(&typo, "a protocol that is no scheme must not start a server");
        assert!(err.contains("`Http`, `http`, `Https`, `https`"), "{}", err);
    }

    // omitting the auth section means: no authentication required (see for example the demo image)
    #[test]
    fn a_config_without_an_authentication_section_parses() {
        let open = config_file(&VALID.replace(
            "[authentication.login]\n        alice = \"...passwordhash...\"",
            "",
        ));

        let config = read_config(open.path().to_str().unwrap()).unwrap();
        assert!(config.authentication.is_none());
        assert!(config.public().is_empty());
    }

    // if authentication was misspelled that would make the entire server public by accident.
    // thats why unknown keys can not be allowed in the config.
    #[test]
    fn a_misspelled_section_refuses_to_parse() {
        let typo = config_file(&VALID.replace("[authentication.login]", "[authentcation.login]"));

        let err = refusal(&typo, "a misspelled section must not open the server");
        assert!(err.contains("unknown field `authentcation`"), "{}", err);
    }

    // An entry in the public array must name a library
    #[test]
    fn a_public_entry_that_names_no_library_refuses_to_parse() {
        for entry in [r#""/**""#, r#""/*/cover/*""#, r#""/library/tags""#] {
            let broken = config_file(&format!("{}\n[authentication]\npublic = [{}]\n", VALID, entry));

            let err = refusal(&broken, "an entry naming no library must not parse");
            assert!(err.contains("public = "), "{}: {}", entry, err);
        }
    }

    #[test]
    fn a_config_that_is_not_there_is_skipped() {
        let fallback = config_file(VALID);

        let config = find_config(vec![
            Some("/nonexistent/path/to/config.toml".to_string()),
            path_of(&fallback),
        ]);

        assert_eq!(config.unwrap().server.port, 8080);
    }

    #[test]
    fn test_path_error_display() {
        let error = PathError {
            path: String::from("/invalid/path"),
            error: anyhow!("sample error"),
        };
        assert_eq!(format!("{}", error), "/invalid/path : sample error");
    }

    #[test]
    fn test_loading_config_from_env_variable() {
        let valid_toml = r#"
        [server]
        ip = "127.0.0.1"
        port = 8080
        protocol = "http"

        [authentication.login]
        alice = "...reallylonghash..."
        bob = "...relativelylonghash..."

        [authentication]
        public = ["/", "/library"]

        [calibre]
        libraries = {}
        "#;
        let mut tmp_file = NamedTempFile::new().unwrap();
        write!(tmp_file, "{}", valid_toml).unwrap();
        env::set_var("ORCA_CONFIG", tmp_file.path());

        let config = read_config(tmp_file.path().to_str().unwrap());
        assert!(config.is_ok());
        assert_eq!(get().server.ip, "127.0.0.1");
        env::remove_var("ORCA_CONFIG");
    }
}
