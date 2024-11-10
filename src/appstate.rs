use crate::config;
use crate::DatabaseConnection;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub templates: tera::Tera,
    pub config: config::Config,
    pub db: Arc<DatabaseConnection>,
}
