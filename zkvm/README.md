# vibehouse zkVM programs

SP1 guest and host programs for generating execution proofs.

## Architecture

```
zkvm/
├── guest/     SP1 guest program (runs inside zkVM, compiled to RISC-V ELF)
│   └── src/main.rs    Re-executes Ethereum block, commits public values
├── host/      SP1 host program (runs natively, generates proofs)
│   └── src/main.rs    Fetches block data from EL, calls SP1 prover
└── README.md
```

**Guest program**: Runs inside the SP1 zkVM. Uses RSP's client executor to
re-execute an Ethereum block, then commits a 96-byte public values output
(`block_hash || parent_hash || state_root`). The CL verifier checks that
`block_hash` matches the proof's claimed execution block hash.

**Host program**: Runs on the proof generator node (builder or dedicated prover).
Fetches block data and state witness from the EL via RPC, packages it as
`ClientExecutorInput`, and calls the SP1 prover to generate a Groth16 proof.

## Prerequisites

1. Install the SP1 toolchain:
   ```bash
   curl -L https://sp1.succinct.xyz | bash
   sp1up
   ```

2. For GPU-accelerated proving, install CUDA drivers.

## Building the guest program

```bash
cd zkvm/guest
cargo prove build
```

This produces an ELF binary at `elf/riscv32im-succinct-zkvm-elf`.

## Building the host program

```bash
cd zkvm/host
cargo build --release
```

The host build script automatically compiles the guest ELF via `sp1-build`.

## Generating a proof

```bash
# CPU proving (slow, testing only — hours per block)
SP1_PROVER=cpu ./target/release/vibehouse-sp1-host \
    --rpc-url http://localhost:8545 \
    --block-number 100

# GPU proving (fast — minutes per block)
SP1_PROVER=cuda ./target/release/vibehouse-sp1-host \
    --rpc-url http://localhost:8545 \
    --block-number 100

# Succinct Network (remote proving)
SP1_PROVER=network SP1_PRIVATE_KEY=<key> ./target/release/vibehouse-sp1-host \
    --rpc-url http://localhost:8545 \
    --block-number 100
```

Output: `proof_data.bin` containing the proof_data field for an `ExecutionProof`
with `version=2` (SP1 Groth16).

## Proof format

The `proof_data.bin` output matches vibehouse's wire format:

```
[0..32]              vkey_hash (32 bytes, SP1 program verification key hash)
[32..36]             groth16_proof_length (u32 big-endian)
[36..36+proof_len]   groth16_proof_bytes
[36+proof_len..]     sp1_public_values (96 bytes)
```

Public values layout:
```
[0..32]   block_hash    keccak256 of RLP-encoded block header
[32..64]  parent_hash   parent block's hash
[64..96]  state_root    post-execution state root
```

## Integration with vibehouse CL

The CL node does NOT run the host program directly. Instead:

1. **Proof generators** run the host program separately and publish proofs to
   execution proof gossip subnets (or submit via the HTTP API).

2. **CL nodes** verify incoming proofs using `sp1-verifier` (lightweight, CPU-only,
   no SP1 toolchain needed). The verification is in
   `beacon_node/beacon_chain/src/execution_proof_verification.rs`.

3. **Task 20d** will add an async proof generation path inside the CL node itself
   (for nodes with `--generate-execution-proofs`), calling `sp1-sdk` via
   `spawn_blocking`.

## Verification key (vkey)

The guest program's verification key is derived from the compiled ELF binary.
It changes whenever the guest program code changes (any code change produces a
different ELF, which produces a different vkey).

The vkey hash must match between the proof generator and the CL verifier. In the
current implementation, the CL verifier accepts any vkey hash embedded in the
proof (the `vkey_hash` field in proof_data). A future task will pin the accepted
vkey hash(es) to the chain spec or a configuration file.

## Not part of main workspace

These crates are intentionally NOT members of vibehouse's main Cargo workspace:

- **Guest**: Targets `riscv32im-succinct-zkvm-elf` (not native x86/ARM)
- **Host**: Depends on `sp1-sdk` (~heavy, GPU support, network client) which
  would pollute the main workspace dependency tree
- Both depend on RSP and reth, which have their own dependency trees

The main workspace only depends on `sp1-verifier` (lightweight, `no_std`).
