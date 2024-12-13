use rustls::{pki_types::PrivateKeyDer, ServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};
use std::fs::File;
use std::io::BufReader;

pub fn load_rustls_config(cert_path: &str, key_path: &str) -> rustls::ServerConfig {

    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();

    let config = ServerConfig::builder().with_no_client_auth();

    let cert_file = &mut BufReader::new(File::open(cert_path).expect("Could not open cert file"));
    let key_file = &mut BufReader::new(File::open(key_path).expect("Could not open key file"));

    let cert_chain = certs(cert_file).collect::<Result<Vec<_>, _>>().unwrap();
    let mut keys = pkcs8_private_keys(key_file)
        .map(|key| key.map(PrivateKeyDer::Pkcs8))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    // exit if no keys could be parsed
    if keys.is_empty() {
        eprintln!("Could not locate PKCS 8 private keys.");
        std::process::exit(1);
    }

    config.with_single_cert(cert_chain, keys.remove(0)).unwrap()
}
