//! Defines the JSON representation of the "cipher" module.
//!
//! This file **MUST NOT** contain any logic beyond what is required to serialize/deserialize the
//! data structures. Specifically, there should not be any actual crypto logic in this file.

use super::hex_bytes::HexBytes;
use serde::{Deserialize, Serialize};

/// Used for ensuring that serde only decodes valid cipher functions.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum CipherFunction {
    Aes128Ctr,
}

impl From<CipherFunction> for String {
    fn from(from: CipherFunction) -> String {
        match from {
            CipherFunction::Aes128Ctr => "aes-128-ctr".into(),
        }
    }
}

impl TryFrom<String> for CipherFunction {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_ref() {
            "aes-128-ctr" => Ok(CipherFunction::Aes128Ctr),
            other => Err(format!("Unsupported cipher function: {}", other)),
        }
    }
}

/// Cipher module representation.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CipherModule {
    pub function: CipherFunction,
    pub params: Cipher,
    pub message: HexBytes,
}

/// Parameters for AES128 with ctr mode.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Aes128Ctr {
    pub iv: HexBytes,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Cipher {
    Aes128Ctr(Aes128Ctr),
}

impl Cipher {
    pub fn function(&self) -> CipherFunction {
        match &self {
            Cipher::Aes128Ctr(_) => CipherFunction::Aes128Ctr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- CipherFunction TryFrom/Into ---

    #[test]
    fn cipher_function_aes128ctr_from_string() {
        let f = CipherFunction::try_from("aes-128-ctr".to_string()).unwrap();
        assert_eq!(f, CipherFunction::Aes128Ctr);
    }

    #[test]
    fn cipher_function_unsupported() {
        let err = CipherFunction::try_from("aes-256-gcm".to_string()).unwrap_err();
        assert!(err.contains("Unsupported"));
    }

    #[test]
    fn cipher_function_empty_string() {
        let err = CipherFunction::try_from(String::new()).unwrap_err();
        assert!(err.contains("Unsupported"));
    }

    #[test]
    fn cipher_function_into_string() {
        assert_eq!(String::from(CipherFunction::Aes128Ctr), "aes-128-ctr");
    }

    #[test]
    fn cipher_function_roundtrip() {
        let s: String = CipherFunction::Aes128Ctr.into();
        let recovered = CipherFunction::try_from(s).unwrap();
        assert_eq!(recovered, CipherFunction::Aes128Ctr);
    }

    #[test]
    fn cipher_function_serde_roundtrip() {
        let f = CipherFunction::Aes128Ctr;
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "\"aes-128-ctr\"");
        let recovered: CipherFunction = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered, f);
    }

    // --- Cipher::function ---

    #[test]
    fn cipher_function_accessor() {
        let iv = vec![0u8; 16];
        let cipher = Cipher::Aes128Ctr(Aes128Ctr { iv: iv.into() });
        assert_eq!(cipher.function(), CipherFunction::Aes128Ctr);
    }
}
