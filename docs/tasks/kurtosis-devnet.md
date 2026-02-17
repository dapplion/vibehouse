# Kurtosis Testing — epbs-devnet-0

## Objective
Participate in epbs-devnet-0 (launch target: Feb 18, 2026). Run vibehouse + geth in kurtosis, verify gloas fork works.

## Status: NOT STARTED

### Specs
- Consensus specs: `v1.7.0-alpha.2` (we're already on this ✅)
- Only self-built payloads tested (no builder marketplace)
- Fork name: `gloas`, fork epoch: 1, preset: `minimal`
- Reference: https://notes.ethereum.org/@ethpandaops/epbs-devnet-0

### Tasks

#### Step 1: Run reference config (no vibehouse)
- [ ] Install kurtosis CLI
- [ ] Run devnet config with consensoor + geth
- [ ] Confirm: chain starts, reaches epoch 1 (gloas fork), finalizes

#### Step 2: Build vibehouse docker image
- [ ] Create Dockerfile (or adapt upstream lighthouse Dockerfile)
- [ ] Build local image: `docker build -t vibehouse:local .`
- [ ] Verify image runs

#### Step 3: Run kurtosis with vibehouse
- [ ] vibehouse CL + geth EL
- [ ] Does it boot? Connect to geth?
- [ ] Does it produce blocks pre-fork?
- [ ] Does it survive gloas fork at epoch 1?
- [ ] Does it produce gloas blocks (self-built payloads)?
- [ ] Does chain finalize post-fork?

#### Step 4: Fix issues
- [ ] Boot/startup failures
- [ ] Fork transition failures
- [ ] Block production failures
- [ ] State transition failures

#### Step 5: Multi-node
- [ ] Run alongside other CL clients
- [ ] Verify cross-client interop
- [ ] Test with 2+ vibehouse nodes

### Blockers
1. Block production — needs self-build (DONE ✅)
2. Payload envelope import — needs full state transition wiring
3. EL integration — `newPayload` for gloas payloads

### Kurtosis config
```yaml
participants:
  - el_type: geth
    el_image: ethpandaops/geth:epbs-devnet-0
    cl_type: lighthouse
    cl_image: vibehouse:local
    count: 1
network_params:
  gloas_fork_epoch: 1
  preset: minimal
additional_services:
  - dora
snooper_enabled: true
global_log_level: debug
dora_params:
  image: ethpandaops/dora:gloas-support
```

## Progress log

### 2026-02-17: devnet readiness assessment & clippy cleanup
- **Compilation**: `cargo check --release` passes cleanly
- **EF spec tests**: 78/78 + 136/136 (fake_crypto) all pass
- **Clippy**: Fixed 80+ lint errors across 32 files to pass `cargo clippy --release --workspace -- -D warnings`. Key fixes: safe_arith in consensus code, indexing → `.get()`, collapsed if statements, redundant closures.
- **Dockerfile**: Reviewed — correctly builds lighthouse binary and copies to `/usr/local/bin/lighthouse`
- **Kurtosis configs**: `kurtosis/vibehouse-epbs.yaml` and `kurtosis/epbs-devnet-0.yaml` exist and look correct
- **No runtime blockers**: No `todo!()`, `unimplemented!()`, or `GloasNotImplemented` remain in the codebase
- **Blocker**: Docker not available on current machine — need Docker to build image and run kurtosis
- **Next**: Install Docker, build image, run kurtosis solo devnet
