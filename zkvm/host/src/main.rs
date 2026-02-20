//! vibehouse SP1 host program for execution proof generation.
//!
//! This program runs on the proof generator node (builder or dedicated prover).
//! It fetches block data from the EL via RPC, prepares the state witness,
//! and calls the SP1 prover to generate a Groth16 proof.
//!
//! # Usage
//!
//! ```bash
//! # CPU proving (slow, testing only)
//! SP1_PROVER=cpu vibehouse-sp1-host --rpc-url http://localhost:8545 --block-number 100
//!
//! # GPU proving (fast, production)
//! SP1_PROVER=cuda vibehouse-sp1-host --rpc-url http://localhost:8545 --block-number 100
//!
//! # Succinct Network (remote proving)
//! SP1_PROVER=network SP1_PRIVATE_KEY=<key> vibehouse-sp1-host --rpc-url http://localhost:8545 --block-number 100
//! ```
//!
//! The output is a binary file containing the proof_data field for an
//! ExecutionProof with version=2 (SP1 Groth16):
//! ```text
//! [0..32]              vkey_hash (32 bytes)
//! [32..36]             groth16_proof_length (u32 big-endian)
//! [36..36+proof_len]   groth16_proof_bytes
//! [36+proof_len..]     sp1_public_values (96 bytes)
//! ```

use clap::Parser;
use eyre::Result;
use sp1_sdk::{ProverClient, SP1Stdin};
use std::path::PathBuf;

/// The guest program ELF, compiled by the build script.
const GUEST_ELF: &[u8] = include_bytes!("../../guest/elf/riscv32im-succinct-zkvm-elf");

#[derive(Parser)]
#[command(name = "vibehouse-sp1-host")]
#[command(about = "Generate SP1 Groth16 execution proofs for vibehouse")]
struct Cli {
    /// EL JSON-RPC URL
    #[arg(long)]
    rpc_url: String,

    /// Block number to prove
    #[arg(long)]
    block_number: u64,

    /// Output file for the proof_data bytes
    #[arg(long, default_value = "proof_data.bin")]
    output: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    println!("Fetching block {} from {}", cli.block_number, cli.rpc_url);

    // Step 1: Fetch the block and state witness from the EL.
    // RSP's host executor handles this — it calls eth_getBlockByNumber,
    // debug_executionWitness (or traces the block to gather accessed state),
    // and packages everything into a ClientExecutorInput.
    let host_executor = rsp_host_executor::HostExecutor::new(
        alloy_provider::ProviderBuilder::new()
            .connect(&cli.rpc_url)
            .await?,
    );

    let input = host_executor
        .execute(cli.block_number)
        .await
        .map_err(|e| eyre::eyre!("failed to fetch execution input: {e}"))?;

    // Serialize the input for the guest program.
    let input_bytes = bincode::serialize(&input)?;
    println!(
        "Execution input: {} bytes ({} KB)",
        input_bytes.len(),
        input_bytes.len() / 1024
    );

    // Step 2: Set up the SP1 prover client and prepare stdin.
    let client = ProverClient::from_env();
    let (pk, vk) = client.setup(GUEST_ELF);

    let mut stdin = SP1Stdin::new();
    stdin.write_vec(input_bytes);

    // Step 3: Generate the Groth16 proof.
    println!("Starting Groth16 proof generation...");
    let proof = client
        .prove(&pk, &stdin)
        .groth16()
        .run()
        .map_err(|e| eyre::eyre!("proof generation failed: {e}"))?;

    println!("Proof generated successfully");

    // Step 4: Verify the proof locally before writing output.
    client
        .verify(&proof, &vk)
        .map_err(|e| eyre::eyre!("local verification failed: {e}"))?;

    println!("Local verification passed");

    // Step 5: Package the proof into vibehouse's proof_data format.
    // Layout: vkey_hash (32) || proof_len (4) || groth16_proof || public_values
    let vkey_hash = vk.bytes32();
    let groth16_bytes = proof.bytes();
    let public_values = proof.public_values.as_slice();

    let proof_len = groth16_bytes.len() as u32;
    let mut proof_data = Vec::with_capacity(32 + 4 + groth16_bytes.len() + public_values.len());
    proof_data.extend_from_slice(vkey_hash.as_ref());
    proof_data.extend_from_slice(&proof_len.to_be_bytes());
    proof_data.extend_from_slice(&groth16_bytes);
    proof_data.extend_from_slice(public_values);

    std::fs::write(&cli.output, &proof_data)?;

    println!(
        "Proof data written to {} ({} bytes)",
        cli.output.display(),
        proof_data.len()
    );
    println!("vkey_hash: 0x{}", hex::encode(vkey_hash.as_ref()));
    println!("groth16_proof: {} bytes", groth16_bytes.len());
    println!("public_values: {} bytes", public_values.len());

    // Print the proven block hash from public values.
    if public_values.len() >= 32 {
        println!("proven block_hash: 0x{}", hex::encode(&public_values[..32]));
    }

    Ok(())
}
