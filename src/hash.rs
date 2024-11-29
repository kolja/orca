use crypto_hash::{hex_digest, Algorithm};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use regex::Regex;
use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct LoginData {
    pub login: String,
    pub password: String,
    pub salt: String,
}

#[derive(Debug)]
pub struct LoginDataError {
    details: String,
}

impl LoginDataError {
    fn new(msg: &str) -> Self {
        LoginDataError {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for LoginDataError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for LoginDataError {
    fn description(&self) -> &str {
        &self.details
    }
}

impl LoginData {
    pub fn new(login_password: &str) -> Result<Self, LoginDataError> {
        let re = Regex::new(r"^([^:]{3,25}):(.{3,25})$").unwrap();
        if !re.is_match(login_password) {
            return Err(LoginDataError::new("Invalid login:password format"));
        }

        let (login, password) = Self::split_by_colon(login_password)?;
        let salt = Self::generate_salt();

        Ok(Self {
            login,
            password,
            salt,
        })
    }

    pub fn new_with_salt(login_password: &str, salt: &str) -> Result<Self, LoginDataError> {
        let re = Regex::new(r"^([^:]{3,25}):(.{3,25})$").unwrap();
        if !re.is_match(login_password) {
            return Err(LoginDataError::new("Invalid login:password format"));
        }

        let (login, password) = Self::split_by_colon(login_password)?;

        Ok(Self {
            login,
            password,
            salt: salt.to_string(),
        })
    }

    fn split_by_colon(login_password: &str) -> Result<(String, String), LoginDataError> {
        let parts: Vec<&str> = login_password.split(':').collect();

        if parts.len() != 2 {
            return Err(LoginDataError::new("Invalid login:password format"));
        }

        Ok((parts[0].to_string(), parts[1].to_string()))
    }

    fn generate_salt() -> String {
        thread_rng()
            .sample_iter(&Alphanumeric)
            .take(20)
            .map(char::from)
            .collect()
    }

    pub fn hash(&self) -> String {
        hex_digest(
            Algorithm::SHA256,
            format!("{}:{}", self.password, self.salt).as_bytes(),
        )[0..20]
            .to_string() // I declare, 20 bytes are enough to avoid collisions
    }
}
