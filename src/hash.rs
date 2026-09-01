use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use anyhow::{Result, anyhow};

pub fn hash(login: &str, password: &str) -> Result<String> {
    if login.len() < 3 || login.len() > 25 || password.len() < 3 || password.len() > 25 {
        return Err(anyhow!("Invalid login or password format"));
    }

    // argon2 salts the password itself, with 16 random bytes
    let argon2 = Argon2::default();
    let hash: PasswordHash = argon2.hash_password(password.as_bytes())
                                   .map_err(|e| anyhow!("Hash error: {:?}", e))?;

    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    Argon2::default().verify_password(password.as_bytes(), hash).is_ok()
}

pub fn encode_auth_data(login: &str, password: &str) -> Result<String> {
    let hash_value = hash(login, password)?;
    Ok(format!(
        "Add this to the [authentication.login] section of your config.toml:\n{} = \"{}\"",
        login, hash_value
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
        assert!(!hash_result.unwrap().is_empty());
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
        assert!(verify_password(password, &hash_result));
    }

    #[test]
    fn test_verify_password_does_not_match() {
        let login = "alice";
        let password = "SecretPassword";
        let hash_result = hash(login, password).unwrap();

        let wrong_password = "WrongPassword";
        assert!(!verify_password(wrong_password, &hash_result));
    }

    #[test]
    fn test_verify_password_refuses_a_hash_it_cannot_read() {
        // What a config holds if someone typed the hash in by hand
        for hash in ["", "secretpassword", "$argon2id$v=19$m=19456,t=2,p=1$notasalt"] {
            assert!(!verify_password("secretpassword", hash), "{}", hash);
        }
    }

    #[test]
    fn test_encode_auth_data() {
        let login = "bob";
        let password = "BobIsCool";
        let encode_result = encode_auth_data(login, password);
        assert!(encode_result.is_ok());
        let encoded_string = encode_result.unwrap();
        assert!(encoded_string.contains("Add this to the [authentication.login] section of your config.toml:"));
        assert!(encoded_string.contains(login));
    }
}
