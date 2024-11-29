use rust_embed::Embed;

#[derive(Embed)]
#[folder = "templates/"]
#[include = "*.xml.tera"]
pub struct Template;
