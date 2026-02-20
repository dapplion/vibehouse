//! vibehouse SP1 guest program for execution proofs.
//!
//! This program runs inside the SP1 zkVM. It re-executes an Ethereum block
//! using RSP's client executor (reth inside the zkVM) and commits a 96-byte
//! public values output: `block_hash || parent_hash || state_root`.
//!
//! The CL verifier checks that `block_hash` matches the proof's claimed
//! execution block hash. If the proof verifies, the block's execution is
//! cryptographically proven correct without the verifier needing EL state.
//!
//! # Build
//!
//! Requires the SP1 toolchain (`sp1up && sp1up`):
//! ```bash
//! cd zkvm/guest
//! cargo prove build
//! ```
//!
//! The output ELF is written to `elf/riscv32im-succinct-zkvm-elf`.

#![no_main]
sp1_zkvm::entrypoint!(main);

use rsp_client_executor::{executor::EthClientExecutor, io::EthClientExecutorInput};
use std::sync::Arc;

/// Size of the public values output (3 x 32-byte hashes).
const PUBLIC_VALUES_SIZE: usize = 96;

pub fn main() {
    // Read the block execution input from the host.
    // The host serializes ClientExecutorInput with bincode and writes it
    // via SP1Stdin::write_vec(). We read the raw bytes and deserialize.
    let input_bytes = sp1_zkvm::io::read_vec();
    let input: EthClientExecutorInput =
        bincode::deserialize(&input_bytes).expect("failed to deserialize executor input");

    // Create the Ethereum executor with the chain's genesis config.
    let executor = EthClientExecutor::eth(
        Arc::new((&input.genesis).try_into().expect("invalid genesis")),
        input.custom_beneficiary,
    );

    // Re-execute the block inside the zkVM.
    // This validates all transactions, computes state changes, and derives
    // the post-execution state root. If any transaction is invalid or the
    // state root doesn't match, execution panics (proof generation fails).
    let header = executor.execute(input).expect("block execution failed");

    // Commit the 96-byte public values: block_hash || parent_hash || state_root.
    //
    // We use commit_slice() (not commit<T>()) to get exact byte layout control
    // without bincode framing. The CL verifier parses these bytes directly as
    // ExecutionProofPublicValues.
    let block_hash = header.hash_slow();
    let mut public_values = [0u8; PUBLIC_VALUES_SIZE];
    public_values[..32].copy_from_slice(block_hash.as_ref());
    public_values[32..64].copy_from_slice(header.parent_hash.as_ref());
    public_values[64..96].copy_from_slice(header.state_root.as_ref());
    sp1_zkvm::io::commit_slice(&public_values);
}
