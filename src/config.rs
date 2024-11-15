
use std::sync::{RwLock, RwLockReadGuard};
use std::fs;
use std::collections::HashMap;
use serde_derive::{Serialize, Deserialize};
use toml;

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

fn read_config(config_file: &str) -> Config {
    let contents = match fs::read_to_string(config_file) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Could not read file `{}`", config_file);
            exit(1);
        }
    };

    let config: Config = match toml::from_str(&contents) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Unable to load data from `{}`: {}", config_file, e);
            exit(1);
        }
    };
    return config;
}

lazy_static! {
    static ref CONFIG: RwLock<Config> = RwLock::new(read_config("config.toml"));
}

pub fn get() -> RwLockReadGuard<'static, Config> {
    CONFIG.read().unwrap()
}
