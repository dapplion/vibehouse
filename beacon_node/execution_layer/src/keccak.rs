use types::Hash256;

pub fn keccak256(bytes: &[u8]) -> Hash256 {
    Hash256::from(alloy_primitives::utils::keccak256(bytes).as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input() {
        // keccak256("") = c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470
        let expected = Hash256::from_slice(
            &hex::decode("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
                .unwrap(),
        );
        assert_eq!(keccak256(&[]), expected);
    }

    #[test]
    fn known_vector() {
        // keccak256("hello") = 1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8
        let expected = Hash256::from_slice(
            &hex::decode("1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8")
                .unwrap(),
        );
        assert_eq!(keccak256(b"hello"), expected);
    }

    #[test]
    fn deterministic() {
        let a = keccak256(b"test input");
        let b = keccak256(b"test input");
        assert_eq!(a, b, "same input should produce same output");
    }

    #[test]
    fn different_inputs_produce_different_hashes() {
        let a = keccak256(b"input A");
        let b = keccak256(b"input B");
        assert_ne!(a, b);
    }
}
