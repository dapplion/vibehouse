use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroize;

type HmacSha256 = Hmac<Sha256>;

/// JWT secret length in bytes.
pub(crate) const JWT_SECRET_LENGTH: usize = 32;

#[derive(Debug)]
pub enum Error {
    InvalidToken,
    InvalidSignature,
    InvalidKey(String),
    Encoding(String),
}

/// Provides wrapper around `[u8; JWT_SECRET_LENGTH]` that implements `Zeroize`.
#[derive(Zeroize, Clone)]
#[zeroize(drop)]
pub struct JwtKey([u8; JWT_SECRET_LENGTH]);

impl JwtKey {
    /// Wrap given slice in `Self`. Returns an error if slice.len() != `JWT_SECRET_LENGTH`.
    pub fn from_slice(key: &[u8]) -> Result<Self, String> {
        if key.len() != JWT_SECRET_LENGTH {
            return Err(format!(
                "Invalid key length. Expected {} got {}",
                JWT_SECRET_LENGTH,
                key.len()
            ));
        }
        let mut res = [0; JWT_SECRET_LENGTH];
        res.copy_from_slice(key);
        Ok(Self(res))
    }

    /// Generate a random secret.
    pub fn random() -> Self {
        Self(rand::rng().random::<[u8; JWT_SECRET_LENGTH]>())
    }

    /// Returns a reference to the underlying byte array.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the hex encoded `String` for the secret.
    pub fn hex_string(&self) -> String {
        hex::encode(self.0)
    }
}

pub(crate) fn strip_prefix(s: &str) -> &str {
    if let Some(stripped) = s.strip_prefix("0x") {
        stripped
    } else {
        s
    }
}

/// Validated JWT token data.
pub(crate) struct TokenData {
    #[cfg_attr(not(test), allow(dead_code))]
    pub claims: Claims,
}

/// Contains the JWT secret and claims parameters.
pub(crate) struct Auth {
    key: [u8; JWT_SECRET_LENGTH],
    id: Option<String>,
    clv: Option<String>,
}

impl Auth {
    pub(crate) fn new(secret: JwtKey, id: Option<String>, clv: Option<String>) -> Self {
        let mut key = [0u8; JWT_SECRET_LENGTH];
        key.copy_from_slice(secret.as_bytes());
        Self { key, id, clv }
    }

    /// Create a new `Auth` struct given the path to the file containing the hex
    /// encoded jwt key.
    #[cfg(test)]
    pub(crate) fn new_with_path(
        jwt_path: std::path::PathBuf,
        id: Option<String>,
        clv: Option<String>,
    ) -> Result<Self, Error> {
        std::fs::read_to_string(&jwt_path)
            .map_err(|e| {
                Error::InvalidKey(format!(
                    "Failed to read JWT secret file {}, error: {e:?}",
                    jwt_path.display()
                ))
            })
            .and_then(|ref s| {
                let secret_bytes = hex::decode(strip_prefix(s.trim_end()))
                    .map_err(|e| Error::InvalidKey(format!("Invalid hex string: {e:?}")))?;
                let secret = JwtKey::from_slice(&secret_bytes).map_err(Error::InvalidKey)?;
                Ok(Self::new(secret, id, clv))
            })
    }

    /// Generate a JWT token with `claims.iat` set to current time.
    pub(crate) fn generate_token(&self) -> Result<String, Error> {
        let claims = self.generate_claims_at_timestamp();
        self.generate_token_with_claims(&claims)
    }

    /// Generate a JWT token with the given claims.
    fn generate_token_with_claims(&self, claims: &Claims) -> Result<String, Error> {
        let header = r#"{"alg":"HS256","typ":"JWT"}"#;
        let header_b64 = URL_SAFE_NO_PAD.encode(header.as_bytes());

        let payload = serde_json::to_vec(claims)
            .map_err(|e| Error::Encoding(format!("Failed to serialize claims: {e}")))?;
        let payload_b64 = URL_SAFE_NO_PAD.encode(&payload);

        let signing_input = format!("{header_b64}.{payload_b64}");

        let mut mac = HmacSha256::new_from_slice(&self.key)
            .map_err(|e| Error::Encoding(format!("HMAC key error: {e}")))?;
        mac.update(signing_input.as_bytes());
        let signature = mac.finalize().into_bytes();
        let signature_b64 = URL_SAFE_NO_PAD.encode(&signature);

        Ok(format!("{signing_input}.{signature_b64}"))
    }

    /// Generate a `Claims` struct with `iat` set to current time
    fn generate_claims_at_timestamp(&self) -> Claims {
        let iat = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before UNIX epoch")
            .as_secs();
        Claims {
            iat,
            id: self.id.clone(),
            clv: self.clv.clone(),
        }
    }

    /// Validate a JWT token given the secret key and return the originally signed `TokenData`.
    pub(crate) fn validate_token(token: &str, secret: &JwtKey) -> Result<TokenData, Error> {
        let parts: Vec<&str> = token.splitn(3, '.').collect();
        if parts.len() != 3 {
            return Err(Error::InvalidToken);
        }

        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let signature = URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|_| Error::InvalidToken)?;

        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| Error::InvalidToken)?;
        mac.update(signing_input.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| Error::InvalidSignature)?;

        let payload_bytes = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| Error::InvalidToken)?;
        let claims: Claims =
            serde_json::from_slice(&payload_bytes).map_err(|_| Error::InvalidToken)?;

        Ok(TokenData { claims })
    }
}

/// Claims struct as defined in <https://github.com/ethereum/execution-apis/blob/main/src/engine/authentication.md#jwt-claims>
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Claims {
    /// issued-at claim. Represented as seconds passed since UNIX_EPOCH.
    iat: u64,
    /// Optional unique identifier for the CL node.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// Optional client version for the CL node.
    #[serde(skip_serializing_if = "Option::is_none")]
    clv: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::DEFAULT_JWT_SECRET;

    #[test]
    fn test_roundtrip() {
        let auth = Auth::new(
            JwtKey::from_slice(&DEFAULT_JWT_SECRET).unwrap(),
            Some("42".into()),
            Some("Vibehouse".into()),
        );
        let claims = auth.generate_claims_at_timestamp();
        let token = auth.generate_token_with_claims(&claims).unwrap();

        assert_eq!(
            Auth::validate_token(&token, &JwtKey::from_slice(&DEFAULT_JWT_SECRET).unwrap())
                .unwrap()
                .claims,
            claims
        );
    }

    #[test]
    fn jwt_key_from_slice_correct_length() {
        let bytes = [0xABu8; JWT_SECRET_LENGTH];
        let key = JwtKey::from_slice(&bytes).unwrap();
        assert_eq!(key.as_bytes(), &bytes);
    }

    #[test]
    fn jwt_key_from_slice_wrong_length() {
        let short = [0u8; 16];
        assert!(JwtKey::from_slice(&short).is_err());

        let long = [0u8; 64];
        assert!(JwtKey::from_slice(&long).is_err());

        let empty: [u8; 0] = [];
        assert!(JwtKey::from_slice(&empty).is_err());
    }

    #[test]
    fn jwt_key_random_is_unique() {
        let k1 = JwtKey::random();
        let k2 = JwtKey::random();
        assert_ne!(k1.as_bytes(), k2.as_bytes());
    }

    #[test]
    fn jwt_key_hex_string() {
        let bytes = [0xABu8; JWT_SECRET_LENGTH];
        let key = JwtKey::from_slice(&bytes).unwrap();
        let hex = key.hex_string();
        assert_eq!(hex.len(), JWT_SECRET_LENGTH * 2);
        assert_eq!(hex, "ab".repeat(JWT_SECRET_LENGTH));
    }

    #[test]
    fn strip_prefix_with_0x() {
        assert_eq!(strip_prefix("0xabcdef"), "abcdef");
    }

    #[test]
    fn strip_prefix_without_0x() {
        assert_eq!(strip_prefix("abcdef"), "abcdef");
    }

    #[test]
    fn strip_prefix_empty() {
        assert_eq!(strip_prefix(""), "");
    }

    #[test]
    fn strip_prefix_only_0x() {
        assert_eq!(strip_prefix("0x"), "");
    }

    #[test]
    fn validate_token_wrong_secret_fails() {
        let secret1 = JwtKey::from_slice(&DEFAULT_JWT_SECRET).unwrap();
        let auth = Auth::new(secret1, None, None);
        let token = auth.generate_token().unwrap();

        let secret2 = JwtKey::random();
        assert!(Auth::validate_token(&token, &secret2).is_err());
    }

    #[test]
    fn validate_token_invalid_string() {
        let secret = JwtKey::from_slice(&DEFAULT_JWT_SECRET).unwrap();
        assert!(Auth::validate_token("not.a.jwt", &secret).is_err());
        assert!(Auth::validate_token("", &secret).is_err());
    }

    #[test]
    fn claims_without_optional_fields() {
        let auth = Auth::new(JwtKey::from_slice(&DEFAULT_JWT_SECRET).unwrap(), None, None);
        let claims = auth.generate_claims_at_timestamp();
        assert!(claims.id.is_none());
        assert!(claims.clv.is_none());
        assert!(claims.iat > 0);

        let token = auth.generate_token_with_claims(&claims).unwrap();
        let decoded =
            Auth::validate_token(&token, &JwtKey::from_slice(&DEFAULT_JWT_SECRET).unwrap())
                .unwrap();
        assert_eq!(decoded.claims, claims);
    }

    #[test]
    fn claims_with_optional_fields() {
        let auth = Auth::new(
            JwtKey::from_slice(&DEFAULT_JWT_SECRET).unwrap(),
            Some("node-1".into()),
            Some("v1.0.0".into()),
        );
        let claims = auth.generate_claims_at_timestamp();
        assert_eq!(claims.id, Some("node-1".into()));
        assert_eq!(claims.clv, Some("v1.0.0".into()));
    }

    #[test]
    fn new_with_path_nonexistent_file() {
        let result = Auth::new_with_path("/nonexistent/path/jwt.hex".into(), None, None);
        assert!(matches!(result, Err(Error::InvalidKey(_))));
    }

    #[test]
    fn new_with_path_valid_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jwt.hex");
        let key = JwtKey::random();
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "0x{}", key.hex_string()).unwrap();

        let auth = Auth::new_with_path(path, Some("test".into()), None).unwrap();
        let token = auth.generate_token().unwrap();
        let decoded = Auth::validate_token(&token, &key).unwrap();
        assert_eq!(decoded.claims.id, Some("test".into()));
    }

    #[test]
    fn new_with_path_no_prefix() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jwt.hex");
        let key = JwtKey::random();
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{}", key.hex_string()).unwrap();

        let auth = Auth::new_with_path(path, None, None).unwrap();
        let token = auth.generate_token().unwrap();
        assert!(Auth::validate_token(&token, &key).is_ok());
    }

    #[test]
    fn new_with_path_invalid_hex() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jwt.hex");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "0xnothex").unwrap();

        let result = Auth::new_with_path(path, None, None);
        assert!(matches!(result, Err(Error::InvalidKey(_))));
    }

    #[test]
    fn new_with_path_wrong_length_hex() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jwt.hex");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "0xaabbcc").unwrap();

        let result = Auth::new_with_path(path, None, None);
        assert!(matches!(result, Err(Error::InvalidKey(_))));
    }

    #[test]
    fn jwt_key_clone() {
        let key = JwtKey::random();
        let cloned = key.clone();
        assert_eq!(key.as_bytes(), cloned.as_bytes());
    }
}
