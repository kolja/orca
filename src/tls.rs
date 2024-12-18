use rustls::{pki_types::PrivateKeyDer, ServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};
use std::fs::File;
use std::io::BufReader;
use anyhow::{Context, Error, Result, bail};

pub fn load_rustls_config(cert_path: &str, key_path: &str) -> Result<ServerConfig, Error> {

    let _ = rustls::crypto::aws_lc_rs::default_provider()
        .install_default().or_else(|_| {
            bail!("Could not install AWS LC provider");
        });

    let config = ServerConfig::builder().with_no_client_auth();

    let cert_file = &mut BufReader::new(File::open(cert_path)
                                        .with_context(|| format!("Could not open cert file"))?);
    let key_file = &mut BufReader::new(File::open(key_path)
                                        .with_context(|| format!("Could not open key file"))?);

    let cert_chain = certs(cert_file).collect::<Result<Vec<_>, _>>().context("Could not parse certificate chain")?;
    let mut keys = pkcs8_private_keys(key_file)
        .map(|key| key.map(PrivateKeyDer::Pkcs8))
        .collect::<Result<Vec<_>, _>>()
        .context("Could not parse PKCS 8 private keys")?;

    if keys.is_empty() {
        bail!("Could not find PKCS 8 private keys.");
    }

    config.with_single_cert(cert_chain, keys.remove(0)).context("Could not load cert/key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_rustls_config_success() {
        let cert_path = "tests/cert.pem";
        let key_path = "tests/key.pem";

        let result = load_rustls_config(cert_path, key_path);

        assert!(result.is_ok());
    }

    #[test]
    fn load_rustls_config_missing_cert() {
        let cert_path = "tests/non_existent_cert.pem";
        let key_path = "tests/key.pem";

        let result = load_rustls_config(cert_path, key_path);

        assert!(result.is_err());
    }

    #[test]
    fn load_rustls_config_missing_key() {
        let cert_path = "tests/cert.pem";
        let key_path = "tests/non_existent_key.pem";

        let result = load_rustls_config(cert_path, key_path);

        assert!(result.is_err());
    }

    #[test]
    fn load_rustls_config_invalid_cert() {
        let cert_path = "tests/invalid.pem";
        let key_path = "tests/key.pem";

        let result = load_rustls_config(cert_path, key_path);

        assert!(result.is_err());
    }

    #[test]
    fn load_rustls_config_invalid_key() {
        let cert_path = "tests/cert.pem";
        let key_path = "tests/invalid.pem";

        let result = load_rustls_config(cert_path, key_path);

        assert!(result.is_err());
    }
}
