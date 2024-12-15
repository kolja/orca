use dirs::home_dir;
use serde_derive::{Deserialize, Serialize};
use std::{fs, env, error::Error, collections::HashMap, process::exit};
use once_cell::sync::Lazy;
use thiserror::Error;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub server: Server,
    pub authentication: HashMap<String, String>,
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

#[derive(Serialize, Deserialize, Clone)]
pub struct Calibre {
    pub libraries: HashMap<String, String>,
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("No config file found anywhere. I checked:\n{0}\nin that order")]
    NotFound(String),
}

fn find_config(paths: Vec<Option<String>>) -> Result<Config, Box<dyn Error>> {
    let paths: Vec<String> = paths.into_iter().flatten().collect();

    paths
        .iter()
        .find_map(|path| match read_config(path) {
            Ok(config) => {
                println!("Config loaded from: {}", path);
                Some(config)
            }
            Err(_) => None,
        })
        .ok_or_else(|| Box::new(ConfigError::NotFound(paths.join("\n"))).into())
}

fn valid_file(config_file: &str) -> bool {
    match fs::metadata(config_file) {
        Ok(metadata) => metadata.is_file(),
        Err(_) => false,
    }
}

pub fn read_config(config_file: &str) -> Result<Config, Box<dyn Error>> {
    if !valid_file(config_file) {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Config file not found",
        )));
    }
    let contents = fs::read_to_string(config_file)?;
    let config = toml::from_str(&contents)?;
    Ok(config)
}

fn load_config() -> Config {
    let conf_from_env: Option<String> = env::var("ORCA_CONFIG").ok();

    let local_conf1: Option<String> = home_dir().and_then(|path_buf| {
        path_buf.to_str().map(|s| format!("{}/.config/orca.toml", s.to_owned()))
    });

    let local_conf2: Option<String> = home_dir().and_then(|path_buf| {
        path_buf.to_str().map(|s| format!("{}/.config/orca/config.toml", s.to_owned()))
    });

    let configs = vec![conf_from_env, local_conf1, local_conf2];

    find_config(configs).unwrap_or_else(|e| {
        eprintln!("{}", e);
        exit(1);
    })
}

static CONFIG: Lazy<Config> = Lazy::new(|| load_config());

pub fn get() -> &'static Config {
    &CONFIG
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_file_exists() {
        assert!(valid_file("tests/orca.http.test.toml"));
    }

    #[test]
    fn test_valid_file_not_exists() {
        assert!(!valid_file("./this.doesnt.exist.toml"));
    }

    #[test]
    fn test_read_config_valid() {
        let config = read_config("tests/orca.http.test.toml");
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.server.ip, "127.0.0.1");
        assert_eq!(config.server.port, 8888);
        match config.server.protocol {
            Protocol::Http => assert!(true),
            _ => assert!(false, "Expected Http protocol"),
        }
    }

    #[test]
    fn test_read_config_invalid() {
        let config = read_config("./invalid_config.toml");
        assert!(config.is_err());
    }

    #[test]
    fn test_read_config_file_not_found() {
        let config = read_config("./non_existent_file.toml");
        assert!(config.is_err());
    }

    #[test]
    fn test_find_config() {
        let configs = vec![
            Some("./non_existent_file.toml".to_string()),
            Some("tests/orca.http.test.toml".to_string())
        ];
        let config = find_config(configs);
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.server.ip, "127.0.0.1");
        assert_eq!(config.server.port, 8888);
    }

    #[test]
    fn test_find_config_not_found() {
        let configs = vec![
            Some("./dosnt.exist.toml".to_string()),
            Some("./dosnt.exist.either.toml".to_string())
        ];
        let config = find_config(configs);
        assert!(config.is_err());
    }

    #[test]
    fn test_load_config() {
        env::set_var("ORCA_CONFIG", "tests/orca.http.test.toml");
        let config = load_config();
        assert_eq!(config.server.ip, "127.0.0.1");
        assert_eq!(config.server.port, 8888);
        env::remove_var("ORCA_CONFIG");
    }

    #[test]
    fn test_get_config() {
        env::set_var("ORCA_CONFIG", "tests/orca.http.test.toml");
        let config = get();
        assert_eq!(config.server.ip, "127.0.0.1");
        assert_eq!(config.server.port, 8888);
        env::remove_var("ORCA_CONFIG");
    }
}
