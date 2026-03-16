//! This library provides a wrapper around several BLS implementations to provide
//! Vibehouse-specific functionality.
//!
//! This crate should not perform direct cryptographic operations, instead it should do these via
//! external libraries. However, seeing as it is an interface to a real cryptographic library, it
//! may contain logic that affects the outcomes of cryptographic operations.
//!
//! A source of complexity in this crate is that *multiple* BLS implementations (a.k.a. "backends")
//! are supported via compile-time flags. There are three backends supported via features:
//!
//! - `supranational`: the pure-assembly, highly optimized version from the `blst` crate.
//! - `fake_crypto`: an always-returns-valid implementation that is only useful for testing
//!   scenarios which intend to *ignore* real cryptography.
//!
//! This crate uses traits to reduce code-duplication between the two implementations. For example,
//! the `GenericPublicKey` struct exported from this crate is generic across the `TPublicKey` trait
//! (i.e., `PublicKey<TPublicKey>`). `TPublicKey` is implemented by all three backends (see the
//! `impls.rs` module).

#[macro_use]
mod macros;
mod generic_aggregate_public_key;
mod generic_aggregate_signature;
mod generic_keypair;
mod generic_public_key;
mod generic_public_key_bytes;
mod generic_secret_key;
mod generic_signature;
mod generic_signature_bytes;
mod generic_signature_set;
mod get_withdrawal_credentials;
mod zeroize_hash;

pub mod impls;

pub use generic_public_key::{
    INFINITY_PUBLIC_KEY, PUBLIC_KEY_BYTES_LEN, PUBLIC_KEY_UNCOMPRESSED_BYTES_LEN,
};
pub use generic_secret_key::SECRET_KEY_BYTES_LEN;
pub use generic_signature::{
    INFINITY_SIGNATURE, INFINITY_SIGNATURE_UNCOMPRESSED, SIGNATURE_BYTES_LEN,
    SIGNATURE_UNCOMPRESSED_BYTES_LEN,
};
pub use get_withdrawal_credentials::get_withdrawal_credentials;
pub use zeroize_hash::ZeroizeHash;

#[cfg(feature = "supranational")]
use blst::BLST_ERROR as BlstError;

pub type Hash256 = fixed_bytes::Hash256;
pub use fixed_bytes::FixedBytesExtended;

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// An error was raised from the Supranational BLST BLS library.
    #[cfg(feature = "supranational")]
    BlstError(BlstError),
    /// The provided bytes were an incorrect length.
    InvalidByteLength { got: usize, expected: usize },
    /// The provided secret key bytes were an incorrect length.
    InvalidSecretKeyLength { got: usize, expected: usize },
    /// The public key represents the point at infinity, which is invalid.
    InvalidInfinityPublicKey,
    /// The secret key is all zero bytes, which is invalid.
    InvalidZeroSecretKey,
}

#[cfg(feature = "supranational")]
impl From<BlstError> for Error {
    fn from(e: BlstError) -> Error {
        Error::BlstError(e)
    }
}

/// Generic implementations which are only generally useful for docs.
pub mod generics {
    pub use crate::generic_aggregate_public_key::GenericAggregatePublicKey;
    pub use crate::generic_aggregate_signature::GenericAggregateSignature;
    pub use crate::generic_keypair::GenericKeypair;
    pub use crate::generic_public_key::GenericPublicKey;
    pub use crate::generic_public_key_bytes::GenericPublicKeyBytes;
    pub use crate::generic_secret_key::GenericSecretKey;
    pub use crate::generic_signature::GenericSignature;
    pub use crate::generic_signature_bytes::GenericSignatureBytes;
    pub use crate::generic_signature_set::WrappedSignature;
}

/// Defines all the fundamental BLS points which should be exported by this crate by making
/// concrete the generic type parameters using the points from some external BLS library (e.g.,BLST).
macro_rules! define_mod {
    ($name: ident, $mod: path) => {
        pub mod $name {
            use $mod as bls_variant;

            use crate::generics::*;

            pub use bls_variant::{SignatureSet, verify_signature_sets};

            pub type PublicKey = GenericPublicKey<bls_variant::PublicKey>;
            pub type PublicKeyBytes = GenericPublicKeyBytes<bls_variant::PublicKey>;
            pub type AggregatePublicKey =
                GenericAggregatePublicKey<bls_variant::PublicKey, bls_variant::AggregatePublicKey>;
            pub type Signature = GenericSignature<bls_variant::PublicKey, bls_variant::Signature>;
            pub type BlsWrappedSignature<'a> = WrappedSignature<
                'a,
                bls_variant::PublicKey,
                bls_variant::AggregatePublicKey,
                bls_variant::Signature,
                bls_variant::AggregateSignature,
            >;
            pub type AggregateSignature = GenericAggregateSignature<
                bls_variant::PublicKey,
                bls_variant::AggregatePublicKey,
                bls_variant::Signature,
                bls_variant::AggregateSignature,
            >;
            pub type SignatureBytes =
                GenericSignatureBytes<bls_variant::PublicKey, bls_variant::Signature>;
            pub type SecretKey = GenericSecretKey<
                bls_variant::Signature,
                bls_variant::PublicKey,
                bls_variant::SecretKey,
            >;
            pub type Keypair = GenericKeypair<
                bls_variant::PublicKey,
                bls_variant::SecretKey,
                bls_variant::Signature,
            >;
        }
    };
}

#[cfg(feature = "supranational")]
define_mod!(blst_implementations, crate::impls::blst::types);
#[cfg(feature = "fake_crypto")]
define_mod!(
    fake_crypto_implementations,
    crate::impls::fake_crypto::types
);

#[cfg(all(feature = "supranational", not(feature = "fake_crypto"),))]
pub use blst_implementations::*;

#[cfg(feature = "fake_crypto")]
pub use fake_crypto_implementations::*;

#[cfg(test)]
mod tests {
    use super::*;
    use ssz::{Decode, Encode};
    use std::collections::HashSet;

    // ====== SecretKey ======

    #[test]
    fn secret_key_random_unique() {
        let sk1 = SecretKey::random();
        let sk2 = SecretKey::random();
        assert_ne!(sk1.serialize().as_bytes(), sk2.serialize().as_bytes());
    }

    #[test]
    fn secret_key_serialize_deserialize_roundtrip() {
        let sk = SecretKey::random();
        let bytes = sk.serialize();
        let sk2 = SecretKey::deserialize(bytes.as_bytes()).unwrap();
        assert_eq!(sk.serialize().as_bytes(), sk2.serialize().as_bytes());
    }

    #[test]
    fn secret_key_deserialize_wrong_length() {
        let err = SecretKey::deserialize(&[1u8; 31]);
        assert!(matches!(
            err,
            Err(Error::InvalidSecretKeyLength {
                got: 31,
                expected: 32
            })
        ));
    }

    #[test]
    fn secret_key_deserialize_zero_rejected() {
        let err = SecretKey::deserialize(&[0u8; 32]);
        assert!(matches!(err, Err(Error::InvalidZeroSecretKey)));
    }

    #[test]
    fn secret_key_deserialize_too_long() {
        let err = SecretKey::deserialize(&[1u8; 33]);
        assert!(matches!(
            err,
            Err(Error::InvalidSecretKeyLength {
                got: 33,
                expected: 32
            })
        ));
    }

    // ====== PublicKey ======

    #[test]
    fn public_key_from_secret_key_deterministic() {
        let sk = SecretKey::random();
        let pk1 = sk.public_key();
        let pk2 = sk.public_key();
        assert_eq!(pk1, pk2);
    }

    #[test]
    fn public_key_serialize_deserialize_roundtrip() {
        let sk = SecretKey::random();
        let pk = sk.public_key();
        let bytes = pk.serialize();
        let pk2 = PublicKey::deserialize(&bytes).unwrap();
        assert_eq!(pk, pk2);
    }

    #[test]
    fn public_key_infinity_rejected() {
        let err = PublicKey::deserialize(&generic_public_key::INFINITY_PUBLIC_KEY);
        assert!(matches!(err, Err(Error::InvalidInfinityPublicKey)));
    }

    #[test]
    fn public_key_wrong_length() {
        let err = PublicKey::deserialize(&[0u8; 47]);
        assert!(err.is_err());
    }

    #[test]
    fn public_key_equality_and_hash() {
        let sk = SecretKey::random();
        let pk1 = sk.public_key();
        let pk2 = sk.public_key();

        assert_eq!(pk1, pk2);

        let mut set = HashSet::new();
        set.insert(pk1.serialize());
        assert!(set.contains(&pk2.serialize()));
    }

    #[test]
    fn public_key_different_keys_not_equal() {
        let pk1 = SecretKey::random().public_key();
        let pk2 = SecretKey::random().public_key();
        assert_ne!(pk1, pk2);
    }

    #[test]
    fn public_key_ssz_roundtrip() {
        let pk = SecretKey::random().public_key();
        let encoded = pk.as_ssz_bytes();
        assert_eq!(encoded.len(), PUBLIC_KEY_BYTES_LEN);
        let decoded = PublicKey::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(pk, decoded);
    }

    #[test]
    fn public_key_hex_string() {
        let pk = SecretKey::random().public_key();
        let hex = pk.as_hex_string();
        assert!(hex.starts_with("0x"));
        assert_eq!(hex.len(), 2 + PUBLIC_KEY_BYTES_LEN * 2);
    }

    #[test]
    fn public_key_compress_decompress_roundtrip() {
        let pk = SecretKey::random().public_key();
        let pk_bytes = pk.compress();
        let pk2 = pk_bytes.decompress().unwrap();
        assert_eq!(pk, pk2);
    }

    // ====== PublicKeyBytes ======

    #[test]
    fn public_key_bytes_empty() {
        let empty = PublicKeyBytes::empty();
        assert_eq!(empty.serialize(), [0u8; PUBLIC_KEY_BYTES_LEN]);
    }

    #[test]
    fn public_key_bytes_from_public_key() {
        let pk = SecretKey::random().public_key();
        let pk_bytes = PublicKeyBytes::from(&pk);
        assert_eq!(pk_bytes.serialize(), pk.serialize());
    }

    #[test]
    fn public_key_bytes_deserialize_wrong_length() {
        let err = PublicKeyBytes::deserialize(&[0u8; 47]);
        assert!(matches!(
            err,
            Err(Error::InvalidByteLength {
                got: 47,
                expected: 48
            })
        ));
    }

    #[test]
    fn public_key_bytes_deserialize_correct_length() {
        let bytes = [0u8; PUBLIC_KEY_BYTES_LEN];
        let pk_bytes = PublicKeyBytes::deserialize(&bytes).unwrap();
        assert_eq!(pk_bytes.serialize(), bytes);
    }

    #[test]
    fn public_key_bytes_ssz_roundtrip() {
        let pk = SecretKey::random().public_key();
        let pk_bytes = PublicKeyBytes::from(&pk);
        let encoded = pk_bytes.as_ssz_bytes();
        let decoded = PublicKeyBytes::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(pk_bytes, decoded);
    }

    // ====== Signature ======

    #[test]
    fn signature_sign_and_verify() {
        let sk = SecretKey::random();
        let pk = sk.public_key();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sig = sk.sign(msg);
        assert!(sig.verify(&pk, msg));
    }

    #[test]
    fn signature_verify_wrong_message() {
        let sk = SecretKey::random();
        let pk = sk.public_key();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let wrong_msg = Hash256::from_slice(&[43u8; 32]);
        let sig = sk.sign(msg);
        assert!(!sig.verify(&pk, wrong_msg));
    }

    #[test]
    fn signature_verify_wrong_key() {
        let sk = SecretKey::random();
        let wrong_pk = SecretKey::random().public_key();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sig = sk.sign(msg);
        assert!(!sig.verify(&wrong_pk, msg));
    }

    #[test]
    fn signature_serialize_deserialize_roundtrip() {
        let sk = SecretKey::random();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sig = sk.sign(msg);
        let bytes = sig.serialize();
        let sig2 = Signature::deserialize(&bytes).unwrap();
        assert_eq!(sig, sig2);
    }

    #[test]
    fn signature_empty() {
        let sig = Signature::empty();
        assert!(sig.is_empty());
        assert!(!sig.is_infinity());
        assert_eq!(sig.serialize(), [0u8; SIGNATURE_BYTES_LEN]);
    }

    #[test]
    fn signature_empty_verify_fails() {
        let pk = SecretKey::random().public_key();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sig = Signature::empty();
        assert!(!sig.verify(&pk, msg));
    }

    #[test]
    fn signature_infinity() {
        let sig = Signature::infinity().unwrap();
        assert!(sig.is_infinity());
        assert!(!sig.is_empty());
    }

    #[test]
    fn signature_deserialize_all_zeros_is_empty() {
        let sig = Signature::deserialize(&[0u8; SIGNATURE_BYTES_LEN]).unwrap();
        assert!(sig.is_empty());
    }

    #[test]
    fn signature_ssz_roundtrip() {
        let sk = SecretKey::random();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sig = sk.sign(msg);
        let encoded = sig.as_ssz_bytes();
        assert_eq!(encoded.len(), SIGNATURE_BYTES_LEN);
        let decoded = Signature::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(sig, decoded);
    }

    // ====== AggregateSignature ======

    #[test]
    fn aggregate_signature_empty() {
        let agg = AggregateSignature::empty();
        assert!(agg.is_empty());
        assert!(!agg.is_infinity());
    }

    #[test]
    fn aggregate_signature_infinity() {
        let agg = AggregateSignature::infinity();
        assert!(agg.is_infinity());
        assert!(!agg.is_empty());
    }

    #[test]
    fn aggregate_signature_single_sig() {
        let sk = SecretKey::random();
        let pk = sk.public_key();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sig = sk.sign(msg);

        let mut agg = AggregateSignature::infinity();
        agg.add_assign(&sig);

        assert!(agg.fast_aggregate_verify(msg, &[&pk]));
    }

    #[test]
    fn aggregate_signature_multiple_sigs() {
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sks: Vec<SecretKey> = (0..3).map(|_| SecretKey::random()).collect();
        let pks: Vec<PublicKey> = sks.iter().map(|sk| sk.public_key()).collect();
        let pk_refs: Vec<&PublicKey> = pks.iter().collect();

        let mut agg = AggregateSignature::infinity();
        for sk in &sks {
            agg.add_assign(&sk.sign(msg));
        }

        assert!(agg.fast_aggregate_verify(msg, &pk_refs));
    }

    #[test]
    fn aggregate_signature_wrong_message() {
        let msg = Hash256::from_slice(&[42u8; 32]);
        let wrong_msg = Hash256::from_slice(&[43u8; 32]);
        let sk = SecretKey::random();
        let pk = sk.public_key();

        let mut agg = AggregateSignature::infinity();
        agg.add_assign(&sk.sign(msg));

        assert!(!agg.fast_aggregate_verify(wrong_msg, &[&pk]));
    }

    #[test]
    fn aggregate_signature_empty_pubkeys_returns_false() {
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sk = SecretKey::random();
        let mut agg = AggregateSignature::infinity();
        agg.add_assign(&sk.sign(msg));

        assert!(!agg.fast_aggregate_verify(msg, &[]));
    }

    #[test]
    fn aggregate_signature_serialize_deserialize() {
        let sk = SecretKey::random();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let mut agg = AggregateSignature::infinity();
        agg.add_assign(&sk.sign(msg));

        let bytes = agg.serialize();
        let agg2 = AggregateSignature::deserialize(&bytes).unwrap();
        assert_eq!(agg, agg2);
    }

    #[test]
    fn aggregate_signature_from_single_signature() {
        let sk = SecretKey::random();
        let pk = sk.public_key();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sig = sk.sign(msg);

        let agg = AggregateSignature::from(&sig);
        assert!(agg.fast_aggregate_verify(msg, &[&pk]));
    }

    #[test]
    fn aggregate_signature_add_assign_aggregate() {
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sk1 = SecretKey::random();
        let sk2 = SecretKey::random();
        let pk1 = sk1.public_key();
        let pk2 = sk2.public_key();

        let mut agg1 = AggregateSignature::infinity();
        agg1.add_assign(&sk1.sign(msg));

        let mut agg2 = AggregateSignature::infinity();
        agg2.add_assign(&sk2.sign(msg));

        agg1.add_assign_aggregate(&agg2);
        assert!(agg1.fast_aggregate_verify(msg, &[&pk1, &pk2]));
    }

    #[test]
    fn eth_fast_aggregate_verify_empty_pubkeys_infinity_sig() {
        let msg = Hash256::from_slice(&[42u8; 32]);
        let agg = AggregateSignature::infinity();
        assert!(agg.eth_fast_aggregate_verify(msg, &[]));
    }

    #[test]
    fn eth_fast_aggregate_verify_empty_pubkeys_non_infinity() {
        let msg = Hash256::from_slice(&[42u8; 32]);
        let agg = AggregateSignature::empty();
        assert!(!agg.eth_fast_aggregate_verify(msg, &[]));
    }

    #[test]
    fn aggregate_verify_different_messages() {
        let msg1 = Hash256::from_slice(&[42u8; 32]);
        let msg2 = Hash256::from_slice(&[43u8; 32]);
        let sk1 = SecretKey::random();
        let sk2 = SecretKey::random();
        let pk1 = sk1.public_key();
        let pk2 = sk2.public_key();

        let mut agg = AggregateSignature::infinity();
        agg.add_assign(&sk1.sign(msg1));
        agg.add_assign(&sk2.sign(msg2));

        assert!(agg.aggregate_verify(&[msg1, msg2], &[&pk1, &pk2]));
    }

    #[test]
    fn aggregate_verify_empty_messages() {
        let agg = AggregateSignature::infinity();
        assert!(!agg.aggregate_verify(&[], &[]));
    }

    #[test]
    fn aggregate_verify_mismatched_lengths() {
        let msg = Hash256::from_slice(&[42u8; 32]);
        let pk = SecretKey::random().public_key();
        let agg = AggregateSignature::infinity();
        assert!(!agg.aggregate_verify(&[msg, msg], &[&pk]));
    }

    // ====== Keypair ======

    #[test]
    fn keypair_random() {
        let kp = Keypair::random();
        let msg = Hash256::from_slice(&[42u8; 32]);
        let sig = kp.sk.sign(msg);
        assert!(sig.verify(&kp.pk, msg));
    }

    #[test]
    fn keypair_pk_matches_sk_public_key() {
        let kp = Keypair::random();
        assert_eq!(kp.pk, kp.sk.public_key());
    }

    // ====== ZeroizeHash ======

    #[test]
    fn zeroize_hash_zero() {
        let zh = ZeroizeHash::zero();
        assert_eq!(zh.as_bytes(), &[0u8; SECRET_KEY_BYTES_LEN]);
    }

    #[test]
    fn zeroize_hash_from_array() {
        let arr = [42u8; SECRET_KEY_BYTES_LEN];
        let zh = ZeroizeHash::from(arr);
        assert_eq!(zh.as_bytes(), &arr);
    }

    #[test]
    fn zeroize_hash_as_mut_bytes() {
        let mut zh = ZeroizeHash::zero();
        zh.as_mut_bytes()[0] = 0xff;
        assert_eq!(zh.as_bytes()[0], 0xff);
    }

    // ====== get_withdrawal_credentials ======

    #[test]
    fn withdrawal_credentials_prefix() {
        let pk = SecretKey::random().public_key();
        let creds = get_withdrawal_credentials(&pk, 0x00);
        assert_eq!(creds.len(), 32);
        assert_eq!(creds[0], 0x00);
    }

    #[test]
    fn withdrawal_credentials_different_prefix() {
        let pk = SecretKey::random().public_key();
        let creds_00 = get_withdrawal_credentials(&pk, 0x00);
        let creds_01 = get_withdrawal_credentials(&pk, 0x01);
        assert_eq!(creds_00[0], 0x00);
        assert_eq!(creds_01[0], 0x01);
        assert_eq!(creds_00[1..], creds_01[1..]);
    }

    #[test]
    fn withdrawal_credentials_deterministic() {
        let sk = SecretKey::random();
        let pk = sk.public_key();
        let creds1 = get_withdrawal_credentials(&pk, 0x00);
        let creds2 = get_withdrawal_credentials(&pk, 0x00);
        assert_eq!(creds1, creds2);
    }
}
