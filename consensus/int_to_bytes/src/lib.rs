/// Returns `int` as little-endian bytes with a length of 1.
pub fn int_to_bytes1(int: u8) -> [u8; 1] {
    [int]
}

/// Returns `int` as little-endian bytes with a length of 2.
pub fn int_to_bytes2(int: u16) -> [u8; 2] {
    int.to_le_bytes()
}

/// Returns `int` as little-endian bytes with a length of 3.
///
/// An `Option` is returned as Rust does not support a native
/// `u24` type.
///
/// The Eth 2.0 specification uses `int.to_bytes(2, 'little')`, which throws an error if `int`
/// doesn't fit within 3 bytes. The specification relies upon implicit asserts for some validity
/// conditions, so we ensure the calling function is aware of the error condition as opposed to
/// hiding it with a modulo.
pub fn int_to_bytes3(int: u32) -> Option<[u8; 3]> {
    if int < 2_u32.pow(3 * 8) {
        let le = int.to_le_bytes();
        Some([le[0], le[1], le[2]])
    } else {
        None
    }
}

/// Returns `int` as little-endian bytes with a length of 4.
pub fn int_to_bytes4(int: u32) -> [u8; 4] {
    int.to_le_bytes()
}

/// Returns `int` as little-endian bytes with a length of 8.
pub fn int_to_bytes8(int: u64) -> [u8; 8] {
    int.to_le_bytes()
}

/// Returns `int` as little-endian bytes with a length of 32.
pub fn int_to_bytes32(int: u64) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&int.to_le_bytes());
    bytes
}

/// Returns `int` as little-endian bytes with a length of 48.
pub fn int_to_bytes48(int: u64) -> [u8; 48] {
    let mut bytes = [0; 48];
    bytes[..8].copy_from_slice(&int.to_le_bytes());
    bytes
}

/// Returns `int` as little-endian bytes with a length of 96.
pub fn int_to_bytes96(int: u64) -> [u8; 96] {
    let mut bytes = [0; 96];
    bytes[..8].copy_from_slice(&int.to_le_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, io::prelude::*, path::PathBuf};
    use yaml_rust2::yaml;

    #[test]
    fn int_to_bytes3_returns_none() {
        assert_eq!(int_to_bytes3(2_u32.pow(24)), None);
    }

    #[test]
    fn test_vectors() {
        /*
         * Test vectors are generated here:
         *
         * https://github.com/ethereum/eth2.0-test-generators
         */
        let mut file = {
            let mut file_path_buf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            file_path_buf.push("src/specs/test_vector_int_to_bytes.yml");

            File::open(file_path_buf).unwrap()
        };

        let mut yaml_str = String::new();

        file.read_to_string(&mut yaml_str).unwrap();

        let docs = yaml::YamlLoader::load_from_str(&yaml_str).unwrap();
        let doc = &docs[0];
        let test_cases = doc["test_cases"].as_vec().unwrap();

        for test_case in test_cases {
            let byte_length = test_case["byte_length"].as_i64().unwrap() as u64;
            let int = test_case["int"].as_i64().unwrap() as u64;
            let bytes_string = test_case["bytes"].clone().into_string().unwrap();
            let bytes = hex::decode(bytes_string.replace("0x", "")).unwrap();

            match byte_length {
                1 => assert_eq!(&int_to_bytes1(int as u8)[..], &bytes[..]),
                2 => assert_eq!(&int_to_bytes2(int as u16)[..], &bytes[..]),
                3 => assert_eq!(&int_to_bytes3(int as u32).unwrap()[..], &bytes[..],),
                4 => assert_eq!(&int_to_bytes4(int as u32)[..], &bytes[..]),
                8 => assert_eq!(&int_to_bytes8(int)[..], &bytes[..]),
                32 => assert_eq!(&int_to_bytes32(int)[..], &bytes[..]),
                48 => assert_eq!(&int_to_bytes48(int)[..], &bytes[..]),
                96 => assert_eq!(&int_to_bytes96(int)[..], &bytes[..]),
                _ => panic!("Unknown byte length in test vector."),
            }
        }
    }
}
