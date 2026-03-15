use serde::{Deserialize, Serialize};
use types::Slot;

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct CustodyInfo {
    pub earliest_custodied_data_column_slot: Slot,
    #[serde(with = "serde_utils::quoted_u64")]
    pub custody_group_count: u64,
    #[serde(with = "serde_utils::quoted_u64_vec")]
    pub custody_columns: Vec<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let info = CustodyInfo {
            earliest_custodied_data_column_slot: Slot::new(100),
            custody_group_count: 4,
            custody_columns: vec![0, 1, 2, 3],
        };
        let json = serde_json::to_string(&info).unwrap();
        let decoded: CustodyInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, decoded);
    }

    #[test]
    fn quoted_u64_serialization() {
        let info = CustodyInfo {
            earliest_custodied_data_column_slot: Slot::new(0),
            custody_group_count: 42,
            custody_columns: vec![10, 20],
        };
        let json = serde_json::to_string(&info).unwrap();
        // custody_group_count should be quoted as string
        assert!(json.contains("\"42\""));
        // custody_columns should be quoted as strings
        assert!(json.contains("\"10\""));
        assert!(json.contains("\"20\""));
    }

    #[test]
    fn empty_columns() {
        let info = CustodyInfo {
            earliest_custodied_data_column_slot: Slot::new(0),
            custody_group_count: 0,
            custody_columns: vec![],
        };
        let json = serde_json::to_string(&info).unwrap();
        let decoded: CustodyInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, decoded);
    }

    #[test]
    fn debug_format() {
        let info = CustodyInfo {
            earliest_custodied_data_column_slot: Slot::new(5),
            custody_group_count: 2,
            custody_columns: vec![1],
        };
        let dbg = format!("{:?}", info);
        assert!(dbg.contains("CustodyInfo"));
    }
}
