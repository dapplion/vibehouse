use crate::{
    ChainSpec, Domain, EthSpec, Fork, Hash256, PublicKey, SecretKey, Signature, SignedRoot, Slot,
};
use ethereum_hashing::hash_fixed;
use safe_arith::{ArithError, SafeArith};
use serde::{Deserialize, Serialize};
use std::cmp;

#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SelectionProof(Signature);

impl SelectionProof {
    pub fn new<E: EthSpec>(
        slot: Slot,
        secret_key: &SecretKey,
        fork: &Fork,
        genesis_validators_root: Hash256,
        spec: &ChainSpec,
    ) -> Self {
        let domain = spec.get_domain(
            slot.epoch(E::slots_per_epoch()),
            Domain::SelectionProof,
            fork,
            genesis_validators_root,
        );
        let message = slot.signing_root(domain);

        Self(secret_key.sign(message))
    }

    /// Returns the "modulo" used for determining if a `SelectionProof` elects an aggregator.
    pub fn modulo(committee_len: usize, spec: &ChainSpec) -> Result<u64, ArithError> {
        Ok(cmp::max(
            1,
            (committee_len as u64).safe_div(spec.target_aggregators_per_committee)?,
        ))
    }

    pub fn is_aggregator(
        &self,
        committee_len: usize,
        spec: &ChainSpec,
    ) -> Result<bool, ArithError> {
        Self::is_aggregator_sig(&self.0, committee_len, spec)
    }

    /// Check if a signature elects an aggregator without requiring ownership.
    pub fn is_aggregator_sig(
        sig: &Signature,
        committee_len: usize,
        spec: &ChainSpec,
    ) -> Result<bool, ArithError> {
        let modulo = Self::modulo(committee_len, spec)?;
        let signature_hash = hash_fixed(&sig.serialize());
        let signature_hash_int = u64::from_le_bytes(
            signature_hash[0..8]
                .try_into()
                .expect("first 8 bytes of signature should always convert to fixed array"),
        );

        signature_hash_int.safe_rem(modulo).map(|rem| rem == 0)
    }

    pub fn is_aggregator_from_modulo(&self, modulo: u64) -> Result<bool, ArithError> {
        let signature_hash = hash_fixed(&self.0.serialize());
        let signature_hash_int = u64::from_le_bytes(
            signature_hash[0..8]
                .try_into()
                .expect("first 8 bytes of signature should always convert to fixed array"),
        );

        signature_hash_int.safe_rem(modulo).map(|rem| rem == 0)
    }

    pub fn verify<E: EthSpec>(
        &self,
        slot: Slot,
        pubkey: &PublicKey,
        fork: &Fork,
        genesis_validators_root: Hash256,
        spec: &ChainSpec,
    ) -> bool {
        let domain = spec.get_domain(
            slot.epoch(E::slots_per_epoch()),
            Domain::SelectionProof,
            fork,
            genesis_validators_root,
        );
        let message = slot.signing_root(domain);

        self.0.verify(pubkey, message)
    }
}

impl From<SelectionProof> for Signature {
    fn from(from: SelectionProof) -> Signature {
        from.0
    }
}

impl From<Signature> for SelectionProof {
    fn from(sig: Signature) -> Self {
        Self(sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FixedBytesExtended, MinimalEthSpec};

    type E = MinimalEthSpec;

    fn minimal_spec() -> ChainSpec {
        ChainSpec::minimal()
    }

    #[test]
    fn modulo_returns_one_for_small_committee() {
        let spec = minimal_spec();
        // committee_len < target_aggregators_per_committee → max(1, 0) = 1
        let m = SelectionProof::modulo(1, &spec).unwrap();
        assert_eq!(m, 1);
    }

    #[test]
    fn modulo_returns_one_for_zero_committee() {
        let spec = minimal_spec();
        // 0 / 16 = 0, max(1, 0) = 1
        let m = SelectionProof::modulo(0, &spec).unwrap();
        assert_eq!(m, 1);
    }

    #[test]
    fn modulo_exact_division() {
        let spec = minimal_spec();
        // target_aggregators_per_committee = 16, so 64 / 16 = 4
        let m = SelectionProof::modulo(64, &spec).unwrap();
        assert_eq!(m, 4);
    }

    #[test]
    fn modulo_with_remainder() {
        let spec = minimal_spec();
        // 100 / 16 = 6 (integer division)
        let m = SelectionProof::modulo(100, &spec).unwrap();
        assert_eq!(m, 6);
    }

    #[test]
    fn modulo_equals_target() {
        let spec = minimal_spec();
        // 16 / 16 = 1
        let m = SelectionProof::modulo(16, &spec).unwrap();
        assert_eq!(m, 1);
    }

    #[test]
    fn modulo_large_committee() {
        let spec = minimal_spec();
        // 1024 / 16 = 64
        let m = SelectionProof::modulo(1024, &spec).unwrap();
        assert_eq!(m, 64);
    }

    #[test]
    fn is_aggregator_from_modulo_one_always_true() {
        // Any value mod 1 == 0, so always aggregator
        let secret_key = SecretKey::random();
        let sig = secret_key.sign(Hash256::zero());
        let proof = SelectionProof::from(sig);
        assert!(proof.is_aggregator_from_modulo(1).unwrap());
    }

    #[test]
    fn is_aggregator_from_modulo_zero_errors() {
        let secret_key = SecretKey::random();
        let sig = secret_key.sign(Hash256::zero());
        let proof = SelectionProof::from(sig);
        assert!(proof.is_aggregator_from_modulo(0).is_err());
    }

    #[test]
    fn new_and_verify_roundtrip() {
        let spec = minimal_spec();
        let secret_key = SecretKey::random();
        let pubkey = secret_key.public_key();
        let slot = Slot::new(42);
        let fork = Fork::default();
        let genesis_validators_root = Hash256::zero();

        let proof =
            SelectionProof::new::<E>(slot, &secret_key, &fork, genesis_validators_root, &spec);

        assert!(proof.verify::<E>(slot, &pubkey, &fork, genesis_validators_root, &spec));
    }

    #[test]
    fn verify_wrong_slot_fails() {
        let spec = minimal_spec();
        let secret_key = SecretKey::random();
        let pubkey = secret_key.public_key();
        let slot = Slot::new(42);
        let fork = Fork::default();
        let genesis_validators_root = Hash256::zero();

        let proof =
            SelectionProof::new::<E>(slot, &secret_key, &fork, genesis_validators_root, &spec);

        // Different slot should fail verification
        assert!(!proof.verify::<E>(
            Slot::new(43),
            &pubkey,
            &fork,
            genesis_validators_root,
            &spec
        ));
    }

    #[test]
    fn verify_wrong_key_fails() {
        let spec = minimal_spec();
        let secret_key = SecretKey::random();
        let other_key = SecretKey::random();
        let slot = Slot::new(42);
        let fork = Fork::default();
        let genesis_validators_root = Hash256::zero();

        let proof =
            SelectionProof::new::<E>(slot, &secret_key, &fork, genesis_validators_root, &spec);

        assert!(!proof.verify::<E>(
            slot,
            &other_key.public_key(),
            &fork,
            genesis_validators_root,
            &spec
        ));
    }

    #[test]
    fn is_aggregator_deterministic() {
        let spec = minimal_spec();
        let secret_key = SecretKey::random();
        let slot = Slot::new(1);
        let fork = Fork::default();

        let proof = SelectionProof::new::<E>(slot, &secret_key, &fork, Hash256::zero(), &spec);

        let result1 = proof.is_aggregator(128, &spec).unwrap();
        let result2 = proof.is_aggregator(128, &spec).unwrap();
        assert_eq!(result1, result2);
    }

    #[test]
    fn into_signature_roundtrip() {
        let secret_key = SecretKey::random();
        let sig = secret_key.sign(Hash256::zero());
        let sig_bytes = sig.serialize();
        let proof = SelectionProof::from(sig);
        let recovered: Signature = proof.into();
        assert_eq!(recovered.serialize(), sig_bytes);
    }
}
