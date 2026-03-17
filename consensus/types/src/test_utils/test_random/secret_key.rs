use super::*;

impl TestRandom for SecretKey {
    fn random_for_test(rng: &mut impl RngCore) -> Self {
        // Generate a deterministic index from the RNG, then derive a valid BLS secret key
        // using the interop keypairs KDF (hash + reduce mod curve order).
        let index = rng.next_u32() as usize;
        let sk_bytes = eth2_interop_keypairs::be_private_key(index);
        SecretKey::deserialize(&sk_bytes)
            .expect("interop keypair should produce a valid secret key")
    }
}
