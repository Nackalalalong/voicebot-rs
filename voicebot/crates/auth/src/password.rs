use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use tokio::task;

use crate::error::{AuthError, Result};

pub async fn hash(password: &str) -> Result<String> {
    let password = password.to_owned();
    task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| AuthError::Hashing(e.to_string()))
    })
    .await
    .map_err(|e| AuthError::Hashing(e.to_string()))?
}

pub async fn verify(password: &str, hash: &str) -> Result<bool> {
    let password = password.to_owned();
    let hash = hash.to_owned();
    task::spawn_blocking(move || {
        let parsed_hash =
            PasswordHash::new(&hash).map_err(|e| AuthError::Hashing(e.to_string()))?;
        Ok::<bool, AuthError>(
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed_hash)
                .is_ok(),
        )
    })
    .await
    .map_err(|e| AuthError::Hashing(e.to_string()))?
}
