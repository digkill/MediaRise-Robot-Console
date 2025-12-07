//! JWT утилиты

use anyhow::Context;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub device_id: String,
    pub client_id: String,
    pub exp: usize,
    pub iat: usize,
}

impl Claims {
    pub fn new(device_id: String, client_id: String, expires_in_seconds: i64) -> Self {
        let now = Utc::now();
        Self {
            device_id,
            client_id,
            exp: (now + Duration::seconds(expires_in_seconds)).timestamp() as usize,
            iat: now.timestamp() as usize,
        }
    }
}

pub fn generate_jwt_token(
    device_id: &str,
    client_id: &str,
    secret: &str,
) -> anyhow::Result<String> {
    let claims = Claims::new(device_id.to_string(), client_id.to_string(), 3600 * 24); // 24 hours

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_ref()),
    )
    .context("Failed to encode JWT token")
}

pub fn verify_jwt_token(token: &str, secret: &str) -> anyhow::Result<Claims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_ref()),
        &validation,
    )
    .context("Failed to decode JWT token")?;

    Ok(token_data.claims)
}
