use clap::Parser;
use orca::config;
use orca::hash::LoginData;
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
    if let Some(auth_data) = encode_auth_data(&args) {
        println!("{}", auth_data);
        std::process::exit(0);
    }

    let config = config::get();

    run_server(create_app(config.clone())).await
}

fn encode_auth_data(args: &Cli) -> Option<String> {
    args.login_password.as_ref().and_then(|login_password| {
        LoginData::new(login_password).ok().map(|data| {
            format!(
                "{}\n{} = \"{}:{}\"",
                "Add this to the [Authentication] section of your config.toml:",
                data.login,
                data.hash(),
                data.salt
            )
        })
    })
}

