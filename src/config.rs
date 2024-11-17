
use std::sync::{RwLock, RwLockReadGuard};
use std::sync::LazyLock;
use std::env;
use std::fs;
use std::collections::HashMap;
use std::error::Error;
use serde_derive::{Serialize, Deserialize};
use toml;
use dirs::home_dir;
use thiserror::Error;

use std::process::exit;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub server: Server,
    pub authentication: Authentication,
    pub calibre: Calibre,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Server {
    pub ip: String,
    pub port: u16,
    pub templates: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Authentication {
    pub credentials: Vec<String>,
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

    let paths: Vec<String> = paths.into_iter().filter_map(|p| p).collect();

    paths.iter()
        .find_map(|path| {
            match read_config(path) {
                Ok(config) => {
                    println!("Config loaded from: {}", path);
                    Some(config)
                }
                Err(_) => None,
            }
        })
        .ok_or_else(|| Box::new(ConfigError::NotFound(paths.join("\n"))).into())
}

fn valid_file(config_file: &str) -> bool {
    match fs::metadata(config_file) {
        Ok(metadata) => metadata.is_file(),
        Err(_) => false
    }
}

fn read_config(config_file: &str) -> Result<Config, Box<dyn Error>> {

    if !valid_file(config_file) {
        return Err(Box::new(std::io::Error::new( std::io::ErrorKind::NotFound, "Config file not found")));
    }

    let contents = fs::read_to_string(config_file)?;
    let config = toml::from_str(&contents)?;
    Ok(config)
}

static CONFIG: LazyLock<RwLock<Config>> = LazyLock::new(|| {

    let conf_from_env: Option<String> = env::var("ORCA_CONFIG").ok();

    let local_conf1: Option<String> = home_dir()
        .and_then(|path_buf| path_buf.to_str()
            .map(|s| format!("{}/.config/orca.toml", s.to_owned())));

    let local_conf2: Option<String> = home_dir()
        .and_then(|path_buf| path_buf.to_str()
            .map(|s| format!("{}/.config/orca/config.toml", s.to_owned())));

    let configs = vec![
        conf_from_env,
        local_conf1,
        local_conf2,
    ];

    return RwLock::new(find_config(configs)
        .unwrap_or_else(|e| {
            eprintln!("{}", e);
            exit(1);
        }))
});

pub fn get() -> RwLockReadGuard<'static, Config> {
    CONFIG.read().unwrap()
}
