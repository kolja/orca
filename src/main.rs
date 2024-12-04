use clap::Parser;
use std::process::exit;
use orca::config;
use orca::hash::encode_auth_data;
use orca::{create_app, run_server};

#[derive(Parser, Debug)]
#[clap(
    author = "Kolja Wilcke",
    version = env!("CARGO_PKG_VERSION"),
    about = "A simple OPDS server for Calibre libraries"
)]
struct Cli {
    #[arg(long = "hash", value_name = "login:password")]
    login_password: Option<String>,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {

    // if povided: Print the hash of the login:password string and exit
    let args = Cli::parse();
    args.login_password.as_ref()
        .and_then(|login_password| encode_auth_data(login_password))
        .map(|auth_data| {
            println!("{}", auth_data);
            exit(0);
        });

    let config = config::get();

    run_server(create_app(config.clone())).await
}

