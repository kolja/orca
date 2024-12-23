use clap::Parser;
use std::process::exit;
use orca::{config, create_app, run_server, hash::encode_auth_data};

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
        .and_then(|login_password| {
            let (login, password) = login_password.split_once(":")?;
            encode_auth_data(login, password)
        })
        .map(|auth_data| {
            println!("{}", auth_data);
            exit(0);
        });

    let config = config::get();

    run_server(create_app(&config)).await
}

