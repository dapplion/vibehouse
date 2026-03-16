//! Defines the JSON representation of the "kdf" module.
//!
//! This file **MUST NOT** contain any logic beyond what is required to serialize/deserialize the
//! data structures. Specifically, there should not be any actual crypto logic in this file.

use super::hex_bytes::HexBytes;
use crate::DKLEN;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// KDF module representation.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KdfModule {
    pub function: KdfFunction,
    pub params: Kdf,
    pub message: EmptyString,
}

/// Used for ensuring serde only decodes an empty string.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EmptyString;

impl From<EmptyString> for String {
    fn from(_from: EmptyString) -> String {
        "".into()
    }
}

impl TryFrom<String> for EmptyString {
    type Error = &'static str;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_ref() {
            "" => Ok(Self),
            _ => Err("kdf message must be empty"),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Kdf {
    Scrypt(Scrypt),
    Pbkdf2(Pbkdf2),
}

impl Kdf {
    pub fn function(&self) -> KdfFunction {
        match &self {
            Kdf::Pbkdf2(_) => KdfFunction::Pbkdf2,
            Kdf::Scrypt(_) => KdfFunction::Scrypt,
        }
    }
}

/// PRF for use in `pbkdf2`.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, Default)]
pub enum Prf {
    #[serde(rename = "hmac-sha256")]
    #[default]
    HmacSha256,
}

impl Prf {
    pub fn mac(&self, password: &[u8]) -> impl Mac {
        match &self {
            Prf::HmacSha256 => Hmac::<Sha256>::new_from_slice(password)
                .expect("Could not derive HMAC using SHA256."),
        }
    }
}

/// Parameters for `pbkdf2` key derivation.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pbkdf2 {
    pub c: u32,
    pub dklen: u32,
    pub prf: Prf,
    pub salt: HexBytes,
}

/// Used for ensuring that serde only decodes valid KDF functions.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum KdfFunction {
    Scrypt,
    Pbkdf2,
}

impl From<KdfFunction> for String {
    fn from(from: KdfFunction) -> String {
        match from {
            KdfFunction::Scrypt => "scrypt".into(),
            KdfFunction::Pbkdf2 => "pbkdf2".into(),
        }
    }
}

impl TryFrom<String> for KdfFunction {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_ref() {
            "scrypt" => Ok(KdfFunction::Scrypt),
            "pbkdf2" => Ok(KdfFunction::Pbkdf2),
            other => Err(format!("Unsupported kdf function: {}", other)),
        }
    }
}

/// Parameters for `scrypt` key derivation.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scrypt {
    pub dklen: u32,
    pub n: u32,
    pub r: u32,
    pub p: u32,
    pub salt: HexBytes,
}

impl Scrypt {
    pub fn default_scrypt(salt: Vec<u8>) -> Self {
        Self {
            dklen: DKLEN,
            n: 262144,
            p: 1,
            r: 8,
            salt: salt.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- KdfFunction TryFrom/Into ---

    #[test]
    fn kdf_function_scrypt_from_string() {
        let f = KdfFunction::try_from("scrypt".to_string()).unwrap();
        assert_eq!(f, KdfFunction::Scrypt);
    }

    #[test]
    fn kdf_function_pbkdf2_from_string() {
        let f = KdfFunction::try_from("pbkdf2".to_string()).unwrap();
        assert_eq!(f, KdfFunction::Pbkdf2);
    }

    #[test]
    fn kdf_function_unsupported_string() {
        let err = KdfFunction::try_from("argon2".to_string()).unwrap_err();
        assert!(err.contains("Unsupported"));
    }

    #[test]
    fn kdf_function_into_string() {
        assert_eq!(String::from(KdfFunction::Scrypt), "scrypt");
        assert_eq!(String::from(KdfFunction::Pbkdf2), "pbkdf2");
    }

    #[test]
    fn kdf_function_roundtrip() {
        for func in [KdfFunction::Scrypt, KdfFunction::Pbkdf2] {
            let s: String = func.clone().into();
            let recovered = KdfFunction::try_from(s).unwrap();
            assert_eq!(func, recovered);
        }
    }

    // --- EmptyString ---

    #[test]
    fn empty_string_from_empty() {
        let es = EmptyString::try_from(String::new()).unwrap();
        let s: String = es.into();
        assert_eq!(s, "");
    }

    #[test]
    fn empty_string_from_nonempty_fails() {
        let result = EmptyString::try_from("hello".to_string());
        assert!(result.is_err());
    }

    // --- Kdf::function ---

    #[test]
    fn kdf_function_accessor_pbkdf2() {
        let kdf = Kdf::Pbkdf2(Pbkdf2 {
            c: 262144,
            dklen: 32,
            prf: Prf::HmacSha256,
            salt: vec![0u8; 32].into(),
        });
        assert_eq!(kdf.function(), KdfFunction::Pbkdf2);
    }

    #[test]
    fn kdf_function_accessor_scrypt() {
        let kdf = Kdf::Scrypt(Scrypt::default_scrypt(vec![0u8; 32]));
        assert_eq!(kdf.function(), KdfFunction::Scrypt);
    }

    // --- Scrypt::default_scrypt ---

    #[test]
    fn default_scrypt_params() {
        let salt = vec![1u8; 32];
        let s = Scrypt::default_scrypt(salt.clone());
        assert_eq!(s.dklen, DKLEN);
        assert_eq!(s.n, 262144);
        assert_eq!(s.p, 1);
        assert_eq!(s.r, 8);
        assert_eq!(s.salt.as_bytes(), &salt[..]);
    }

    // --- Prf ---

    #[test]
    fn prf_default_is_hmac_sha256() {
        assert_eq!(Prf::default(), Prf::HmacSha256);
    }

    // --- Serde roundtrip ---

    #[test]
    fn kdf_function_serde_roundtrip() {
        let f = KdfFunction::Scrypt;
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "\"scrypt\"");
        let recovered: KdfFunction = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered, f);
    }

    #[test]
    fn empty_string_serde_roundtrip() {
        let es = EmptyString;
        let json = serde_json::to_string(&es).unwrap();
        assert_eq!(json, "\"\"");
        let recovered: EmptyString = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered, es);
    }

    #[test]
    fn empty_string_serde_nonempty_fails() {
        let result: Result<EmptyString, _> = serde_json::from_str("\"notempty\"");
        assert!(result.is_err());
    }
}
