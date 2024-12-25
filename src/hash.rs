use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2
};
use anyhow::{Result, anyhow};

pub fn hash(login: &str, password: &str) -> Result<String> {
    if login.len() < 3 || login.len() > 25 || password.len() < 3 || password.len() > 25 {
        return Err(anyhow!("Invalid login or password format"));
    }

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2.hash_password(password.as_bytes(), &salt)
                     .map_err(|e| anyhow!("Hash error: {:?}", e))?
                     .to_string();

    Ok(hash)
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(hash).map_err(|err| anyhow!("Hash error: {}", err))?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed_hash).is_ok())
}

pub fn encode_auth_data(login: &str, password: &str) -> Result<String> {
    let hash_value = hash(login, password)?;
    Ok(format!(
        "{}\n{} = \"{}\"",
        "Add this to the [Authentication] section of your config.toml:",
        login,
        hash_value
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_valid_input() {
        let login = "user";
        let password = "strongpassword";
        let hash_result = hash(login, password);
        assert!(hash_result.is_ok());
        assert!(hash_result.unwrap().len() > 0);
    }

    #[test]
    fn test_hash_invalid_input() {
        let login = "al"; // Invalid: fewer than 3 characters
        let password = "pw"; // Invalid: fewer than 3 characters
        let hash_result = hash(login, password);
        assert!(hash_result.is_err());

        let login = "nobodyissupposedtohavethislongofalogin";
        let password = "password123"; // Valid length
        let hash_result = hash(login, password);
        assert!(hash_result.is_err());

        let login = "alice"; // Valid length
        let password = "pw"; // Invalid: fewer than 3 characters
        let hash_result = hash(login, password);
        assert!(hash_result.is_err());
    }

    #[test]
    fn test_verify_password_matches() {
        let login = "bob";
        let password = "cantbeguessed";
        let hash_result = hash(login, password).unwrap();
        let verify_result = verify_password(password, &hash_result);
        assert!(verify_result.is_ok());
        assert!(verify_result.unwrap());
    }

    #[test]
    fn test_verify_password_does_not_match() {
        let login = "alice";
        let password = "SecretPassword";
        let hash_result = hash(login, password).unwrap();

        let wrong_password = "WrongPassword";
        let verify_result = verify_password(wrong_password, &hash_result);
        assert!(verify_result.is_ok());
        assert!(!verify_result.unwrap());
    }

    #[test]
    fn test_encode_auth_data() {
        let login = "bob";
        let password = "BobIsCool";
        let encode_result = encode_auth_data(login, password);
        assert!(encode_result.is_ok());
        let encoded_string = encode_result.unwrap();
        assert!(encoded_string.contains("Add this to the [Authentication] section of your config.toml:"));
        assert!(encoded_string.contains(login));
    }
}
