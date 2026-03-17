use super::*;

impl TestRandom for Signature {
    fn random_for_test(rng: &mut impl RngCore) -> Self {
        // Now that SecretKey::random_for_test is deterministic, produce a real
        // deterministic signature by signing a random message.
        let sk = SecretKey::random_for_test(rng);
        let msg = Hash256::random_for_test(rng);
        sk.sign(msg)
    }
}
