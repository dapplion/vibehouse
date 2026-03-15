use ethereum_hashing::hash_fixed;
use types::{KzgCommitment, VERSIONED_HASH_VERSION_KZG, VersionedHash};

pub fn kzg_commitment_to_versioned_hash(kzg_commitment: &KzgCommitment) -> VersionedHash {
    let mut hashed_commitment = hash_fixed(&kzg_commitment.0);
    hashed_commitment[0] = VERSIONED_HASH_VERSION_KZG;
    VersionedHash::from(hashed_commitment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_hash_starts_with_version_byte() {
        let commitment = KzgCommitment([0u8; 48]);
        let hash = kzg_commitment_to_versioned_hash(&commitment);
        assert_eq!(hash.as_slice()[0], VERSIONED_HASH_VERSION_KZG);
    }

    #[test]
    fn different_commitments_produce_different_hashes() {
        let c1_bytes = [0u8; 48];
        let mut c2_bytes = [0u8; 48];
        c2_bytes[0] = 1;
        let c1 = KzgCommitment(c1_bytes);
        let c2 = KzgCommitment(c2_bytes);
        let h1 = kzg_commitment_to_versioned_hash(&c1);
        let h2 = kzg_commitment_to_versioned_hash(&c2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn same_commitment_produces_same_hash() {
        let commitment = KzgCommitment([42u8; 48]);
        let h1 = kzg_commitment_to_versioned_hash(&commitment);
        let h2 = kzg_commitment_to_versioned_hash(&commitment);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_is_32_bytes() {
        let commitment = KzgCommitment([0u8; 48]);
        let hash = kzg_commitment_to_versioned_hash(&commitment);
        assert_eq!(hash.as_slice().len(), 32);
    }

    #[test]
    fn version_byte_overrides_hash() {
        // Even if the hash of the commitment would start with a different byte,
        // the version byte is always VERSIONED_HASH_VERSION_KZG
        let commitment = KzgCommitment([0xFF; 48]);
        let hash = kzg_commitment_to_versioned_hash(&commitment);
        assert_eq!(hash.as_slice()[0], VERSIONED_HASH_VERSION_KZG);
    }
}
