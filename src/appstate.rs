use crate::config;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use rusqlite::Connection;

#[derive(Clone)]
pub struct AppState {
    pub templates: tera::Tera,
    pub config: config::Config,
    pub db: HashMap<String, Arc<Mutex<Connection>>>,
}
