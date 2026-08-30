use crate::config::Config;
use crate::landing::{self, Landing};
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub templates: tera::Tera,
    pub config: &'static Config,
    pub db: HashMap<String, Arc<Mutex<Connection>>>,
    /// Only concerns the libraries override the default landing page
    pub pages: HashMap<String, Arc<Landing>>,
}

impl AppState {
    /// What the root of one library offers
    pub fn landing(&self, lib: &str) -> &Landing {
        match self.pages.get(lib) {
            Some(page) => page.as_ref(),
            None => landing::default_page(),
        }
    }
}
