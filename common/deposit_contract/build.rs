//! Verifies the ABI and bytecode for the deposit contract are present and have correct checksums.
//!
//! The contract files live in `contracts/` and are checked into the repo.

use sha2::{Digest, Sha256};
use std::env;
use std::path::PathBuf;

const TAG: &str = "v0.12.1";

// Checksums for the production smart contract.
const ABI_CHECKSUM: &str = "e53a64aecdd14f7c46c4134d19500c3184bf083b046347fb14c7828a26f2bff6";
const BYTECODE_CHECKSUM: &str = "ace004b44a9f531bcd47f9d8827b2527c713a2df3af943ac28ecc3df2aa355d6";

// Checksums for the testnet smart contract.
const TESTNET_ABI_CHECKSUM: &str =
    "c9a0a6b3fd48b94193d48c48abad3edcd61eb645d8cdfc9d969d188beb34f5c1";
const TESTNET_BYTECODE_CHECKSUM: &str =
    "2b054e7d134e2d66566ba074c8a18a3a67841d67c8ef6175fc95f1639ee73a89";

fn main() {
    verify_contract(
        "validator_registration.json",
        ABI_CHECKSUM,
        "validator_registration.bytecode",
        BYTECODE_CHECKSUM,
    );
    verify_contract(
        "testnet_validator_registration.json",
        TESTNET_ABI_CHECKSUM,
        "testnet_validator_registration.bytecode",
        TESTNET_BYTECODE_CHECKSUM,
    );
}

fn verify_contract(
    abi_file: &str,
    abi_checksum: &str,
    bytecode_file: &str,
    bytecode_checksum: &str,
) {
    let abi_path = contracts_dir().join(format!("{}_{}", TAG, abi_file));
    let bytecode_path = contracts_dir().join(format!("{}_{}", TAG, bytecode_file));

    let abi_bytes = std::fs::read(&abi_path).unwrap_or_else(|e| {
        panic!(
            "deposit contract ABI not found at {}: {}. These files should be checked into the repo.",
            abi_path.display(),
            e
        )
    });
    verify_checksum(&abi_bytes, abi_checksum);

    let bytecode_bytes = std::fs::read(&bytecode_path).unwrap_or_else(|e| {
        panic!(
            "deposit contract bytecode not found at {}: {}. These files should be checked into the repo.",
            bytecode_path.display(),
            e
        )
    });
    verify_checksum(&bytecode_bytes, bytecode_checksum);
}

fn verify_checksum(bytes: &[u8], expected_checksum: &str) {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    let checksum = hex::encode(&result[..]);
    assert_eq!(
        &checksum, expected_checksum,
        "Checksum {} did not match {}",
        checksum, expected_checksum
    );
}

fn contracts_dir() -> PathBuf {
    env::var("CARGO_MANIFEST_DIR")
        .expect("should know manifest dir")
        .parse::<PathBuf>()
        .expect("should parse manifest dir as path")
        .join("contracts")
}
