#!/usr/bin/env bash
set -euo pipefail

# Bounded devnet lifecycle for vibehouse.
# Build -> clean old enclave -> start -> poll beacon API -> teardown.
#
# Usage: scripts/kurtosis-run.sh [--no-build] [--no-teardown] [--stateless]
#
# Flags:
#   --no-build     Skip Docker image build (use existing vibehouse:local)
#   --no-teardown  Leave enclave running after completion (for inspection)
#   --stateless    Use mixed stateless+proof-generator config (vibehouse-stateless.yaml)
#
# Logs: each run writes to /tmp/kurtosis-runs/<RUN_ID>/ with separate files:
#   build.log       — cargo build + docker build output
#   kurtosis.log    — kurtosis run output (enclave startup)
#   health.log      — beacon API polling results (JSON per poll)
#   dump/           — enclave dump on failure

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ENCLAVE_NAME="vibehouse-devnet"
KURTOSIS_CONFIG="$REPO_ROOT/kurtosis/vibehouse-epbs.yaml"
POLL_INTERVAL=12  # one slot = 6s, poll every 2 slots
TIMEOUT=720       # 12 minutes (epoch 8 ≈ 480s + margin)

# Devnet params (minimal preset)
SLOTS_PER_EPOCH=8
GLOAS_FORK_EPOCH=1
GLOAS_FORK_SLOT=$((GLOAS_FORK_EPOCH * SLOTS_PER_EPOCH))
# Need finalized epoch 8 — sustained chain health across 4 nodes
TARGET_FINALIZED_EPOCH=8

DO_BUILD=true
DO_TEARDOWN=true
STATELESS_MODE=false

for arg in "$@"; do
  case "$arg" in
    --no-build)    DO_BUILD=false ;;
    --no-teardown) DO_TEARDOWN=false ;;
    --stateless)   STATELESS_MODE=true ;;
    *) echo "Unknown flag: $arg"; exit 1 ;;
  esac
done

# Use stateless config when --stateless is set
if [ "$STATELESS_MODE" = true ]; then
  KURTOSIS_CONFIG="$REPO_ROOT/kurtosis/vibehouse-stateless.yaml"
  echo "==> Stateless mode: using $KURTOSIS_CONFIG"
fi

# Set up run directory
RUN_ID="$(date +%Y%m%d-%H%M%S)"
RUN_DIR="/tmp/kurtosis-runs/$RUN_ID"
mkdir -p "$RUN_DIR"
echo "==> Run ID: $RUN_ID"
echo "==> Logs: $RUN_DIR"

cleanup() {
  if [ "$DO_TEARDOWN" = true ]; then
    echo "==> Tearing down enclave $ENCLAVE_NAME..."
    kurtosis enclave rm -f "$ENCLAVE_NAME" 2>/dev/null || true
  else
    echo "==> --no-teardown: enclave $ENCLAVE_NAME left running"
  fi
}

dump_logs() {
  echo "==> Dumping enclave logs to $RUN_DIR/dump/..."
  kurtosis enclave dump "$ENCLAVE_NAME" "$RUN_DIR/dump" 2>/dev/null || true
  echo "==> Logs saved to $RUN_DIR/dump/"
}

# Step 1: Build Docker image
if [ "$DO_BUILD" = true ]; then
  echo "==> Building Docker image (log: $RUN_DIR/build.log)..."
  "$SCRIPT_DIR/build-docker.sh" > "$RUN_DIR/build.log" 2>&1
  tail -1 "$RUN_DIR/build.log"
fi

# Step 2: Clean up old enclave
echo "==> Cleaning up old enclaves..."
kurtosis enclave rm -f "$ENCLAVE_NAME" 2>/dev/null || true
kurtosis clean -a 2>/dev/null || true

# Step 3: Start devnet
echo "==> Starting devnet (log: $RUN_DIR/kurtosis.log)..."
if ! kurtosis run github.com/ethpandaops/ethereum-package@6.0.0 --enclave "$ENCLAVE_NAME" --args-file "$KURTOSIS_CONFIG" > "$RUN_DIR/kurtosis.log" 2>&1; then
  echo "==> FAIL: kurtosis run failed. See $RUN_DIR/kurtosis.log"
  echo "--- last 30 lines ---"
  tail -30 "$RUN_DIR/kurtosis.log"
  cleanup
  exit 1
fi
echo "==> Devnet started. Services:"
tail -30 "$RUN_DIR/kurtosis.log" | grep -E '(RUNNING|STOPPED|Name)' || true

# Step 4: Discover beacon API port
echo "==> Discovering beacon API endpoint..."
BEACON_URL="$(kurtosis port print "$ENCLAVE_NAME" cl-1-lighthouse-geth http)"
echo "    Beacon API: $BEACON_URL"

# Step 5: Poll beacon API for health checks
echo "==> Polling beacon API (timeout: ${TIMEOUT}s, interval: ${POLL_INTERVAL}s)..."
echo "    Target: finalized_epoch >= $TARGET_FINALIZED_EPOCH (gloas fork at epoch $GLOAS_FORK_EPOCH)"
elapsed=0
prev_slot=0
stall_count=0

while [ "$elapsed" -lt "$TIMEOUT" ]; do
  sleep "$POLL_INTERVAL"
  elapsed=$((elapsed + POLL_INTERVAL))

  # Check 1: Is the node synced?
  syncing=$(curl -sf "$BEACON_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
  if [ -z "$syncing" ]; then
    echo "    [${elapsed}s] Beacon API not ready..."
    continue
  fi

  is_syncing=$(echo "$syncing" | jq -r '.data.is_syncing' 2>/dev/null || echo "true")
  head_slot=$(echo "$syncing" | jq -r '.data.head_slot' 2>/dev/null || echo "0")

  # Check 2: Get finality checkpoints
  finality=$(curl -sf "$BEACON_URL/eth/v1/beacon/states/head/finality_checkpoints" 2>/dev/null || echo "")
  finalized_epoch="0"
  justified_epoch="0"
  if [ -n "$finality" ]; then
    finalized_epoch=$(echo "$finality" | jq -r '.data.finalized.epoch' 2>/dev/null || echo "0")
    justified_epoch=$(echo "$finality" | jq -r '.data.current_justified.epoch' 2>/dev/null || echo "0")
  fi

  # Check 3: Get latest header to verify block production
  header=$(curl -sf "$BEACON_URL/eth/v1/beacon/headers/head" 2>/dev/null || echo "")
  header_slot="0"
  if [ -n "$header" ]; then
    header_slot=$(echo "$header" | jq -r '.data.header.message.slot' 2>/dev/null || echo "0")
  fi

  # Log full state
  {
    echo "--- ${elapsed}s ---"
    echo "head_slot=$head_slot header_slot=$header_slot is_syncing=$is_syncing"
    echo "finalized_epoch=$finalized_epoch justified_epoch=$justified_epoch"
    echo "$syncing" | jq . 2>/dev/null || true
    echo "$finality" | jq . 2>/dev/null || true
  } >> "$RUN_DIR/health.log"

  # Compute current epoch
  current_epoch=$((head_slot / SLOTS_PER_EPOCH))
  past_fork="no"
  if [ "$head_slot" -ge "$GLOAS_FORK_SLOT" ]; then
    past_fork="yes"
  fi

  echo "    [${elapsed}s] slot=$head_slot epoch=$current_epoch finalized=$finalized_epoch justified=$justified_epoch syncing=$is_syncing fork=$past_fork"

  # Stall detection: if head_slot hasn't advanced in 3 polls, something is wrong
  if [ "$head_slot" -eq "$prev_slot" ] && [ "$head_slot" -ne "0" ]; then
    stall_count=$((stall_count + 1))
    if [ "$stall_count" -ge 3 ]; then
      echo "==> FAIL: Chain stalled at slot $head_slot for $((stall_count * POLL_INTERVAL))s"
      dump_logs
      cleanup
      exit 1
    fi
  else
    stall_count=0
  fi
  prev_slot="$head_slot"

  # Success: finalized past the gloas fork
  if [ "$finalized_epoch" -ge "$TARGET_FINALIZED_EPOCH" ]; then
    echo "==> SUCCESS: Finalized epoch $finalized_epoch (target was $TARGET_FINALIZED_EPOCH)"
    echo "    Chain progressed through gloas fork and finalized."

    # In stateless mode, also check the stateless node's health
    if [ "$STATELESS_MODE" = true ]; then
      echo "==> Checking stateless node (cl-4-lighthouse-geth)..."
      STATELESS_URL="$(kurtosis port print "$ENCLAVE_NAME" cl-4-lighthouse-geth http 2>/dev/null || echo "")"
      if [ -n "$STATELESS_URL" ]; then
        # Check stateless node is synced to similar head
        stateless_syncing=$(curl -sf "$STATELESS_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
        if [ -n "$stateless_syncing" ]; then
          stateless_head=$(echo "$stateless_syncing" | jq -r '.data.head_slot' 2>/dev/null || echo "0")
          stateless_finalized=$(curl -sf "$STATELESS_URL/eth/v1/beacon/states/head/finality_checkpoints" 2>/dev/null | jq -r '.data.finalized.epoch' 2>/dev/null || echo "0")
          echo "    Stateless node: head_slot=$stateless_head finalized_epoch=$stateless_finalized"

          # Check proof status for the head block on the stateless node
          proof_status=$(curl -sf "$STATELESS_URL/vibehouse/execution_proof_status/head" 2>/dev/null || echo "")
          if [ -n "$proof_status" ]; then
            is_proven=$(echo "$proof_status" | jq -r '.data.is_fully_proven' 2>/dev/null || echo "unknown")
            received_proofs=$(echo "$proof_status" | jq -r '.data.received_proof_subnet_ids' 2>/dev/null || echo "[]")
            echo "    Proof status: is_fully_proven=$is_proven received=$received_proofs"
          else
            echo "    Proof status endpoint not available (may be expected for non-Gloas head)"
          fi

          # Log stateless health
          {
            echo "=== stateless node check ==="
            echo "head_slot=$stateless_head finalized_epoch=$stateless_finalized"
            echo "proof_status=$proof_status"
          } >> "$RUN_DIR/health.log"
        else
          echo "    Warning: stateless node beacon API not responding"
        fi
      else
        echo "    Warning: could not discover stateless node port"
      fi
    fi

    echo "==> Full logs: $RUN_DIR"
    cleanup
    exit 0
  fi
done

echo "==> TIMEOUT: Did not reach finalized epoch $TARGET_FINALIZED_EPOCH within ${TIMEOUT}s"
echo "    Last state: slot=$prev_slot finalized_epoch=$finalized_epoch"
echo "==> Health log: $RUN_DIR/health.log"
dump_logs
cleanup
exit 1
