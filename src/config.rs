
use dirs::home_dir;
use std::{env, fmt, fs, collections::HashMap};
// use regex::Regex;
use serde_derive::{Deserialize, Serialize};
use crate::pattern::Pattern;

use once_cell::sync::Lazy;
use anyhow::{Context, Error, Result, anyhow};

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub server: Server,
    pub authentication: Authentication,
    pub calibre: Calibre,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "protocol")]
pub enum Protocol {
    Http,
    Https {
        cert: String,
        key: String,
    },
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Server {
    pub ip: String,
    pub port: u16,
    #[serde(flatten)]
    pub protocol: Protocol,
}

#[derive(Serialize, Deserialize)]
pub struct Authentication {
    pub login: HashMap<String, String>,
    #[serde(default)]
    pub public: Vec<Pattern>,
}

impl Default for Authentication {
    fn default() -> Self {
        Authentication {
            login: HashMap::new(),
            public: vec![]
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Calibre {
    pub libraries: HashMap<String, String>,
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
        protocol = "Http"

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
    fn test_find_config_multiple_paths() {
        let mut tmp_file1 = NamedTempFile::new().unwrap();
        let invalid_toml = r#"
        [server
        foo = "127.0.0.1"
        port = 8080
        "#;
        write!(tmp_file1, "{}", invalid_toml).unwrap();

        let mut tmp_file2 = NamedTempFile::new().unwrap();
        let valid_toml = r#"
        [server]
        ip = "127.0.0.1"
        port = 8080
        protocol = "Http"

        [authentication.login]
        alice = "...passwordhash..."

        [calibre]
        libraries = {}
        "#;
        write!(tmp_file2, "{}", valid_toml).unwrap();

        let tmp_file1_path = tmp_file1.path().to_str().unwrap().to_string();
        let tmp_file2_path = tmp_file2.path().to_str().unwrap().to_string();
        let config = find_config(vec![Some(tmp_file1_path), Some(tmp_file2_path)]);
        assert!(config.is_ok());
        assert_eq!(config.unwrap().server.ip, "127.0.0.1");
    }

    #[test]
    fn test_valid_file_nonexistent() {
        let path = "/nonexistent_path/to_config.toml";
        assert!(!valid_file(path));
    }

    #[test]
    fn test_find_config_some_valid_paths() {
        let mut tmp_file1 = NamedTempFile::new().unwrap();
        let invalid_toml = r#"
        [server
        foo = "127.0.0.1"
        bar = 8080
        "#;
        write!(tmp_file1, "{}", invalid_toml).unwrap();

        let mut tmp_file2 = NamedTempFile::new().unwrap();
        let valid_toml = r#"
        [server]
        ip = "127.0.0.1"
        port = 8080
        protocol = "Http"

        [authentication.login]
        bob = "...passwordhash..."

        [calibre]
        libraries = {}
        "#;
        write!(tmp_file2, "{}", valid_toml).unwrap();

        let tmp_file1_path = tmp_file1.path().to_str().unwrap().to_string();
        let tmp_file2_path = tmp_file2.path().to_str().unwrap().to_string();
        let config = find_config(vec![Some(tmp_file1_path), Some(tmp_file2_path)]);
        assert!(config.is_ok());
        assert_eq!(config.unwrap().server.ip, "127.0.0.1");
    }

    #[test]
    fn test_protocol_https() {
        let valid_toml = r#"
        [server]
        ip = "127.0.0.1"
        port = 8080
        protocol = "Https"
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
        protocol = "Http"

        [authentication.login]
        alice = "...reallylonghash..."
        bob = "...relativelylonghash..."

        [authentication]
        public = ["/", "/library/*"]

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
