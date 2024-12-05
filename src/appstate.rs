use crate::config::Config;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub templates: tera::Tera,
    pub config: &'static Config,
    pub db: HashMap<String, Arc<Mutex<Connection>>>,
}
