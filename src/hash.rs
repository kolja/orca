use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct LoginData {
    pub login: String,
    pub password: String,
    pub hash: Option<String>,
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

impl Error for LoginDataError {}

impl LoginData {
    pub fn new(login: &str, password: &str) -> Result<Self, LoginDataError> {
        if login.len() < 3 || login.len() > 25 || password.len() < 3 || password.len() > 25 {
            return Err(LoginDataError::new("Invalid login or password format"));
        }

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2.hash_password(password.as_bytes(), &salt)
                         .map_err(|e| LoginDataError::new(&format!("Hash error: {:?}", e)))?
                         .to_string();

        Ok(Self {
            login: login.to_string(),
            password: password.to_string(),
            hash: Some(hash)
        })
    }

    pub fn new_with_hash(login: &str, password: &str, hash: &str) -> Result<Self, LoginDataError> {
        if login.len() < 3 || login.len() > 25 || password.len() < 3 || password.len() > 25 {
            return Err(LoginDataError::new("Invalid login or password format"));
        }

        Ok(Self {
            login: login.to_string(),
            password: password.to_string(),
            hash: Some(hash.to_string())
        })
    }

    pub fn verify_password(&self) -> Result<bool, LoginDataError> {
        if let Some(hash) = &self.hash {
            let parsed_hash = PasswordHash::new(hash).map_err(|err| LoginDataError::new(&format!("Hash error: {}", err)))?;
            Ok(Argon2::default().verify_password(self.password.as_bytes(), &parsed_hash).is_ok())
        } else {
            Err(LoginDataError::new("No hash provided"))
        }
    }
}

pub fn encode_auth_data(login: &str, password: &str) -> Option<String> {
    LoginData::new(login, password).ok().and_then(|data| {
        Some(format!(
            "{}\n{} = \"{}\"",
            "Add this to the [Authentication] section of your config.toml:",
            data.login,
            data.hash.as_ref().unwrap()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_data_valid() {
        let login_data = LoginData::new("alice", "secretpassword").unwrap();

        assert_eq!(login_data.login, "alice");
        assert_eq!(login_data.password, "secretpassword");
        //assert_eq!(login_data.salt.len(), 20);
        //assert!(login_data.salt.chars().all(|c| c.is_alphanumeric()));
    }

    #[test]
    fn invalid_password_returns_error() {

        let result = LoginData::new("alice", "x");

        assert!(result.is_err());
        assert_eq!(result.err().unwrap().to_string(), "Invalid login or password format");
    }

    #[test]
    fn login_data_with_hash() {
        let login = "alice";
        let password = "secretpassword";
        let hash = "$argon2id$v=19$m=19456,t=2,p=1$DCvcgKhnf9SZEi92Dga+cg$kIhFhv2N0YLjr3Ebxi58aFNELZ9jI6OGAmVWmk6Gj1A";
        let login_data = LoginData::new_with_hash(login, password, hash).unwrap();

        assert_eq!(login_data.login, "alice");
        assert_eq!(login_data.password, "secretpassword");
        assert!(login_data.verify_password().is_ok());
    }

    #[test]
    fn encode_auth_data_valid_roundtrip() {
        use regex::Regex;

        let login = "alice";
        let password = "secretpassword";
        let result = encode_auth_data(login, password).unwrap();

        let re = Regex::new(
            r#"Add this to the \[Authentication\] section of your config\.toml:\n(.*) = "(.*)""#
        ).unwrap();

        let captures = re
            .captures(&result)
            .expect("The result does not match the expected format.");

        let extracted_login = &captures[1];
        let extracted_hash = &captures[2];

        let login_data = LoginData::new_with_hash(extracted_login, password, extracted_hash).unwrap();

        assert!(login_data.verify_password().is_ok());
    }

    #[test]
    fn encode_auth_data_invalid() {
        let result = encode_auth_data("jo", "2");
        assert!(result.is_none());
    }
}
