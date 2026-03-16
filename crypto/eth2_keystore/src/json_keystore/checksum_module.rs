//! Defines the JSON representation of the "checksum" module.
//!
//! This file **MUST NOT** contain any logic beyond what is required to serialize/deserialize the
//! data structures. Specifically, there should not be any actual crypto logic in this file.

use super::hex_bytes::HexBytes;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Used for ensuring that serde only decodes valid checksum functions.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ChecksumFunction {
    Sha256,
}

impl From<ChecksumFunction> for String {
    fn from(from: ChecksumFunction) -> String {
        match from {
            ChecksumFunction::Sha256 => "sha256".into(),
        }
    }
}

impl TryFrom<String> for ChecksumFunction {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_ref() {
            "sha256" => Ok(ChecksumFunction::Sha256),
            other => Err(format!("Unsupported checksum function: {}", other)),
        }
    }
}

/// Used for ensuring serde only decodes an empty map.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "Value")]
pub struct EmptyMap;

impl From<EmptyMap> for Value {
    fn from(_from: EmptyMap) -> Value {
        Value::Object(Map::default())
    }
}

impl TryFrom<Value> for EmptyMap {
    type Error = &'static str;

    fn try_from(v: Value) -> Result<Self, Self::Error> {
        match v {
            Value::Object(map) if map.is_empty() => Ok(Self),
            _ => Err("Checksum params must be an empty map"),
        }
    }
}

/// Checksum module for `Keystore`.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecksumModule {
    pub function: ChecksumFunction,
    pub params: EmptyMap,
    pub message: HexBytes,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Sha256Checksum(String);

impl Sha256Checksum {
    pub fn function() -> ChecksumFunction {
        ChecksumFunction::Sha256
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ChecksumFunction TryFrom/Into ---

    #[test]
    fn checksum_function_sha256_from_string() {
        let f = ChecksumFunction::try_from("sha256".to_string()).unwrap();
        assert_eq!(f, ChecksumFunction::Sha256);
    }

    #[test]
    fn checksum_function_unsupported() {
        let err = ChecksumFunction::try_from("md5".to_string()).unwrap_err();
        assert!(err.contains("Unsupported"));
    }

    #[test]
    fn checksum_function_into_string() {
        assert_eq!(String::from(ChecksumFunction::Sha256), "sha256");
    }

    #[test]
    fn checksum_function_roundtrip() {
        let s: String = ChecksumFunction::Sha256.into();
        let recovered = ChecksumFunction::try_from(s).unwrap();
        assert_eq!(recovered, ChecksumFunction::Sha256);
    }

    #[test]
    fn checksum_function_serde_roundtrip() {
        let f = ChecksumFunction::Sha256;
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "\"sha256\"");
        let recovered: ChecksumFunction = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered, f);
    }

    // --- EmptyMap ---

    #[test]
    fn empty_map_from_empty_object() {
        let v = Value::Object(Map::default());
        let em = EmptyMap::try_from(v).unwrap();
        let back: Value = em.into();
        assert_eq!(back, Value::Object(Map::default()));
    }

    #[test]
    fn empty_map_from_nonempty_object_fails() {
        let mut map = Map::new();
        map.insert("key".to_string(), Value::Null);
        let result = EmptyMap::try_from(Value::Object(map));
        assert!(result.is_err());
    }

    #[test]
    fn empty_map_from_non_object_fails() {
        assert!(EmptyMap::try_from(Value::Null).is_err());
        assert!(EmptyMap::try_from(Value::Bool(true)).is_err());
        assert!(EmptyMap::try_from(Value::String("".into())).is_err());
        assert!(EmptyMap::try_from(Value::Array(vec![])).is_err());
    }

    #[test]
    fn empty_map_serde_roundtrip() {
        let em = EmptyMap;
        let json = serde_json::to_string(&em).unwrap();
        assert_eq!(json, "{}");
        let recovered: EmptyMap = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered, em);
    }

    // --- Sha256Checksum ---

    #[test]
    fn sha256_checksum_function() {
        assert_eq!(Sha256Checksum::function(), ChecksumFunction::Sha256);
    }
}
