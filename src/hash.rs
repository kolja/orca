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

    fn split_by_colon(input: &str) -> Result<(String, String), LoginDataError> {

        let parts: Vec<&str> = input.splitn(2, ':').collect();

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

pub fn encode_auth_data(login_password: &str) -> Option<String> {
    LoginData::new(login_password).ok().map(|data| {
        format!(
            "{}\n{} = \"{}:{}\"",
            "Add this to the [Authentication] section of your config.toml:",
            data.login,
            data.hash(),
            data.salt
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_data_valid() {
        let login_password = "alice:secretpassword";
        let login_data = LoginData::new(login_password).unwrap();

        assert_eq!(login_data.login, "alice");
        assert_eq!(login_data.password, "secretpassword");
        assert_eq!(login_data.salt.len(), 20);
        assert!(login_data.salt.chars().all(|c| c.is_alphanumeric()));
    }

    #[test]
    fn invalid_password_returns_error() {
        let invalid_format = "bob";
        let result = LoginData::new(invalid_format);

        assert!(result.is_err());
        assert_eq!(result.err().unwrap().to_string(), "Invalid login:password format");
    }

    #[test]
    fn login_data_with_salt() {
        let login_password = "bob:secret:password";
        let salt = "specificsaltvalue";
        let login_data = LoginData::new_with_salt(login_password, salt).unwrap();

        assert_eq!(login_data.login, "bob");
        assert_eq!(login_data.password, "secret:password");
        assert_eq!(login_data.salt, "specificsaltvalue");
    }

    #[test]
    fn several_login_password_combinations() {
        assert!(LoginData::new_with_salt("bob:secret/password", "random_salt").is_ok());
        assert!(LoginData::new_with_salt("bob:secret:password", "random_salt").is_ok());
        assert!(LoginData::new_with_salt("alice:*/2701/^&@!:", "random_salt").is_ok());
        assert!(LoginData::new_with_salt("alice:::foobar:::", "random_salt").is_ok());
        assert!(LoginData::new_with_salt("bob:x", "random_salt").is_err());
        assert!(LoginData::new_with_salt("al:ice:secret:password", "random_salt").is_err());
    }

    #[test]
    fn login_data_hash() {
        let login_password = "alice:secretpassword";
        let salt = "specificsaltvalue";
        let login_data = LoginData::new_with_salt(login_password, salt).unwrap();

        let expected_hash = hex_digest(
            Algorithm::SHA256,
            format!("{}:{}", "secretpassword", "specificsaltvalue").as_bytes(),
        )[0..20]
            .to_string();

        assert_eq!(login_data.hash(), expected_hash);
    }

    #[test]
    fn encode_auth_data_valid_roundtrip() {
        use regex::Regex;

        let login_password = "alice:secretpassword";
        let result = encode_auth_data(login_password).unwrap();

        let re = Regex::new(
            r#"Add this to the \[Authentication\] section of your config\.toml:\nalice = "([a-f0-9]{20}):([A-Za-z0-9]{20})""#
        ).unwrap();

        let captures = re
            .captures(&result)
            .expect("The result does not match the expected format.");

        let extracted_hash = &captures[1];
        let extracted_salt = &captures[2];

        let login_data = LoginData::new_with_salt(login_password, extracted_salt).unwrap();

        assert_eq!(login_data.hash(), extracted_hash);
    }

    #[test]
    fn encode_auth_data_invalid() {
        let invalid_login_password = "invalidformat";
        let result = encode_auth_data(invalid_login_password);
        assert!(result.is_none());
    }
}
