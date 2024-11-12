use crate::config;
use std::sync::{Arc, Mutex};
use rusqlite::Connection;

#[derive(Clone)]
pub struct AppState {
    pub templates: tera::Tera,
    pub config: config::Config,
    pub db: Arc<Mutex<Connection>>,
}
