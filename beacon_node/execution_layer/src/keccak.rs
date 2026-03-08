use types::Hash256;

pub fn keccak256(bytes: &[u8]) -> Hash256 {
    Hash256::from(alloy_primitives::utils::keccak256(bytes).as_ref())
}
