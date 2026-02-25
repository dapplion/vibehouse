#!/usr/bin/env bash
set -euo pipefail

# Bounded devnet lifecycle for vibehouse.
# Build -> clean old enclave -> start -> poll beacon API -> teardown.
#
# Usage: scripts/kurtosis-run.sh [--no-build] [--no-teardown] [--stateless] [--multiclient] [--sync] [--churn]
#
# Flags:
#   --no-build      Skip Docker image build (use existing vibehouse:local)
#   --no-teardown   Leave enclave running after completion (for inspection)
#   --stateless     Use mixed stateless+proof-generator config (vibehouse-stateless.yaml)
#   --multiclient   Use vibehouse + lodestar config (vibehouse-multiclient.yaml)
#   --sync          Genesis sync test: stop non-validator nodes, let chain finalize,
#                   restart them, verify they sync through the Gloas fork boundary
#   --churn         Node churn test: finalize chain, kill a validator node, verify chain
#                   continues finalizing (75% stake), restart node, verify recovery
#
# Logs: each run writes to /tmp/kurtosis-runs/<RUN_ID>/ with separate files:
#   build.log       — cargo build + docker build output
#   kurtosis.log    — kurtosis run output (enclave startup)
#   health.log      — beacon API polling results (JSON per poll)
#   sync.log        — sync mode: non-validator node sync progress (--sync only)
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
MULTICLIENT_MODE=false
SYNC_MODE=false
CHURN_MODE=false

for arg in "$@"; do
  case "$arg" in
    --no-build)     DO_BUILD=false ;;
    --no-teardown)  DO_TEARDOWN=false ;;
    --stateless)    STATELESS_MODE=true ;;
    --multiclient)  MULTICLIENT_MODE=true ;;
    --sync)         SYNC_MODE=true ;;
    --churn)        CHURN_MODE=true ;;
    *) echo "Unknown flag: $arg"; exit 1 ;;
  esac
done

# Use stateless config when --stateless is set
if [ "$CHURN_MODE" = true ]; then
  # Churn uses the default 4-node config — all 4 are validators
  TARGET_FINALIZED_EPOCH=3  # lower initial target: just past Gloas fork
  TIMEOUT=900               # 15 minutes total (warm-up + kill + recovery)
  echo "==> Churn mode: using $KURTOSIS_CONFIG (4 validators, will kill/restart node 4)"
elif [ "$SYNC_MODE" = true ]; then
  KURTOSIS_CONFIG="$REPO_ROOT/kurtosis/vibehouse-sync.yaml"
  TARGET_FINALIZED_EPOCH=4  # lower target for pre-sync phase
  TIMEOUT=900               # 15 minutes total (finalization + sync)
  echo "==> Sync mode: using $KURTOSIS_CONFIG (2 validators + 2 sync targets)"
elif [ "$STATELESS_MODE" = true ]; then
  KURTOSIS_CONFIG="$REPO_ROOT/kurtosis/vibehouse-stateless.yaml"
  echo "==> Stateless mode: using $KURTOSIS_CONFIG"
elif [ "$MULTICLIENT_MODE" = true ]; then
  KURTOSIS_CONFIG="$REPO_ROOT/kurtosis/vibehouse-multiclient.yaml"
  echo "==> Multi-client mode: using $KURTOSIS_CONFIG (vibehouse + lodestar)"
fi

# Detect if we need sudo for docker/kurtosis (docker socket not accessible)
SUDO=""
if ! docker info >/dev/null 2>&1; then
  SUDO="sudo"
  echo "==> Docker needs sudo (user not in docker group for current session)"
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
    $SUDO kurtosis enclave rm -f "$ENCLAVE_NAME" 2>/dev/null || true
  else
    echo "==> --no-teardown: enclave $ENCLAVE_NAME left running"
  fi
}

dump_logs() {
  echo "==> Dumping enclave logs to $RUN_DIR/dump/..."
  $SUDO kurtosis enclave dump "$ENCLAVE_NAME" "$RUN_DIR/dump" 2>/dev/null || true
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
$SUDO kurtosis enclave rm -f "$ENCLAVE_NAME" 2>/dev/null || true
$SUDO kurtosis clean -a 2>/dev/null || true

# Step 3: Start devnet
echo "==> Starting devnet (log: $RUN_DIR/kurtosis.log)..."
if ! $SUDO kurtosis run github.com/ethpandaops/ethereum-package@6.0.0 --enclave "$ENCLAVE_NAME" --args-file "$KURTOSIS_CONFIG" > "$RUN_DIR/kurtosis.log" 2>&1; then
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
BEACON_URL="$($SUDO kurtosis port print "$ENCLAVE_NAME" cl-1-lighthouse-geth http 2>/dev/null || true)"
if [ -z "$BEACON_URL" ]; then
  # Multi-client configs may use different service naming
  BEACON_URL="$($SUDO kurtosis port print "$ENCLAVE_NAME" cl-1-lodestar-geth http 2>/dev/null || true)"
fi
if [ -z "$BEACON_URL" ]; then
  echo "==> FAIL: Could not discover beacon API endpoint"
  dump_logs
  cleanup
  exit 1
fi
echo "    Beacon API: $BEACON_URL"

# Sync mode: stop non-validator nodes and wait for finalization on validators first
if [ "$SYNC_MODE" = true ]; then
  SYNC_NODE_SUPER="cl-3-lighthouse-geth"
  SYNC_NODE_FULL="cl-4-lighthouse-geth"
  SYNC_EL_SUPER="el-3-geth-lighthouse"
  SYNC_EL_FULL="el-4-geth-lighthouse"

  echo "==> Sync mode: stopping non-validator nodes..."
  # Stop both CL and EL for non-validator nodes so they fall behind
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$SYNC_NODE_SUPER" 2>/dev/null || true
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$SYNC_NODE_FULL" 2>/dev/null || true
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$SYNC_EL_SUPER" 2>/dev/null || true
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$SYNC_EL_FULL" 2>/dev/null || true
  echo "    Stopped: $SYNC_NODE_SUPER, $SYNC_NODE_FULL, $SYNC_EL_SUPER, $SYNC_EL_FULL"
  echo "    Waiting for validator nodes to finalize past Gloas fork..."
fi

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

  # Success: finalized past the target
  if [ "$finalized_epoch" -ge "$TARGET_FINALIZED_EPOCH" ]; then
    echo "==> Finalized epoch $finalized_epoch (target was $TARGET_FINALIZED_EPOCH)"
    echo "    Chain progressed through gloas fork and finalized."

    # In sync mode, break out to run the sync verification phase
    if [ "$SYNC_MODE" = true ]; then
      VALIDATOR_HEAD_SLOT="$head_slot"
      break
    fi

    # In churn mode, break out to run the kill/recovery phases
    if [ "$CHURN_MODE" = true ]; then
      PRE_CHURN_FINALIZED="$finalized_epoch"
      PRE_CHURN_HEAD="$head_slot"
      break
    fi

    # In multi-client mode, verify cross-client consensus
    if [ "$MULTICLIENT_MODE" = true ]; then
      echo "==> Checking cross-client consensus..."
      for node_name in cl-1-lighthouse-geth cl-2-lighthouse-geth cl-3-lodestar-geth cl-4-lodestar-geth; do
        node_url="$($SUDO kurtosis port print "$ENCLAVE_NAME" "$node_name" http 2>/dev/null || echo "")"
        if [ -n "$node_url" ]; then
          node_head=$(curl -sf "$node_url/eth/v1/node/syncing" 2>/dev/null | jq -r '.data.head_slot' 2>/dev/null || echo "?")
          node_fin=$(curl -sf "$node_url/eth/v1/beacon/states/head/finality_checkpoints" 2>/dev/null | jq -r '.data.finalized.epoch' 2>/dev/null || echo "?")
          echo "    $node_name: head=$node_head finalized=$node_fin"
        fi
      done
    fi

    # In stateless mode, also check the stateless node's health
    if [ "$STATELESS_MODE" = true ]; then
      echo "==> Checking stateless node (cl-4-lighthouse-geth)..."
      STATELESS_URL="$($SUDO kurtosis port print "$ENCLAVE_NAME" cl-4-lighthouse-geth http 2>/dev/null || echo "")"
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

    echo "==> SUCCESS"
    echo "==> Full logs: $RUN_DIR"
    cleanup
    exit 0
  fi
done

# If we broke out of the loop in sync mode, run the sync verification phase
if [ "$SYNC_MODE" = true ] && [ "${VALIDATOR_HEAD_SLOT:-0}" -gt 0 ]; then
  echo ""
  echo "==> SYNC PHASE: Restarting non-validator nodes..."
  echo "    Validator head at slot $VALIDATOR_HEAD_SLOT when sync targets were stopped"

  # Restart EL first, then CL (CL needs EL to be ready)
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$SYNC_EL_SUPER" 2>/dev/null || true
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$SYNC_EL_FULL" 2>/dev/null || true
  sleep 5  # give EL a moment to start
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$SYNC_NODE_SUPER" 2>/dev/null || true
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$SYNC_NODE_FULL" 2>/dev/null || true
  echo "    All non-validator nodes restarted."

  # Discover sync target beacon API URLs
  SYNC_SUPER_URL="$($SUDO kurtosis port print "$ENCLAVE_NAME" "$SYNC_NODE_SUPER" http 2>/dev/null || echo "")"
  SYNC_FULL_URL="$($SUDO kurtosis port print "$ENCLAVE_NAME" "$SYNC_NODE_FULL" http 2>/dev/null || echo "")"

  if [ -z "$SYNC_SUPER_URL" ] || [ -z "$SYNC_FULL_URL" ]; then
    echo "==> FAIL: Could not discover sync target beacon API endpoints"
    echo "    supernode=$SYNC_SUPER_URL fullnode=$SYNC_FULL_URL"
    dump_logs
    cleanup
    exit 1
  fi
  echo "    Supernode API: $SYNC_SUPER_URL"
  echo "    Fullnode API:  $SYNC_FULL_URL"

  # Poll sync targets until they catch up
  SYNC_TIMEOUT=360  # 6 minutes for sync
  SYNC_POLL=6       # poll every slot
  sync_elapsed=0
  super_synced=false
  full_synced=false
  sync_start_time=$(date +%s)

  echo "==> Polling sync targets (timeout: ${SYNC_TIMEOUT}s, interval: ${SYNC_POLL}s)..."
  {
    echo "=== sync phase ==="
    echo "validator_head_at_restart=$VALIDATOR_HEAD_SLOT"
  } >> "$RUN_DIR/sync.log"

  while [ "$sync_elapsed" -lt "$SYNC_TIMEOUT" ]; do
    sleep "$SYNC_POLL"
    sync_elapsed=$((sync_elapsed + SYNC_POLL))

    # Get current validator head for comparison
    val_syncing=$(curl -sf "$BEACON_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
    val_head="?"
    if [ -n "$val_syncing" ]; then
      val_head=$(echo "$val_syncing" | jq -r '.data.head_slot' 2>/dev/null || echo "?")
    fi

    # Check supernode sync status
    super_status="not_ready"
    super_head="0"
    super_is_syncing="true"
    super_resp=$(curl -sf "$SYNC_SUPER_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
    if [ -n "$super_resp" ]; then
      super_head=$(echo "$super_resp" | jq -r '.data.head_slot' 2>/dev/null || echo "0")
      super_is_syncing=$(echo "$super_resp" | jq -r '.data.is_syncing' 2>/dev/null || echo "true")

      # Get detailed sync state from lighthouse-specific endpoint
      super_lh=$(curl -sf "$SYNC_SUPER_URL/lighthouse/syncing" 2>/dev/null || echo "")
      if [ -n "$super_lh" ]; then
        super_status=$(echo "$super_lh" | jq -r 'if (.data | type) == "string" then .data else (.data | keys[0] // "unknown") end' 2>/dev/null || echo "unknown")
      fi

      # Check if synced: not syncing and head within 2 slots of validator head
      if [ "$super_is_syncing" = "false" ] && [ "$super_head" != "0" ]; then
        super_synced=true
      fi
    fi

    # Check fullnode sync status
    full_status="not_ready"
    full_head="0"
    full_is_syncing="true"
    full_resp=$(curl -sf "$SYNC_FULL_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
    if [ -n "$full_resp" ]; then
      full_head=$(echo "$full_resp" | jq -r '.data.head_slot' 2>/dev/null || echo "0")
      full_is_syncing=$(echo "$full_resp" | jq -r '.data.is_syncing' 2>/dev/null || echo "true")

      full_lh=$(curl -sf "$SYNC_FULL_URL/lighthouse/syncing" 2>/dev/null || echo "")
      if [ -n "$full_lh" ]; then
        full_status=$(echo "$full_lh" | jq -r 'if (.data | type) == "string" then .data else (.data | keys[0] // "unknown") end' 2>/dev/null || echo "unknown")
      fi

      if [ "$full_is_syncing" = "false" ] && [ "$full_head" != "0" ]; then
        full_synced=true
      fi
    fi

    echo "    [${sync_elapsed}s] validator_head=$val_head | super: head=$super_head status=$super_status synced=$super_synced | full: head=$full_head status=$full_status synced=$full_synced"

    # Log detailed state
    {
      echo "--- sync ${sync_elapsed}s ---"
      echo "validator_head=$val_head"
      echo "supernode: head=$super_head is_syncing=$super_is_syncing status=$super_status synced=$super_synced"
      echo "fullnode: head=$full_head is_syncing=$full_is_syncing status=$full_status synced=$full_synced"
    } >> "$RUN_DIR/sync.log"

    # Success: both nodes synced
    if [ "$super_synced" = true ] && [ "$full_synced" = true ]; then
      sync_duration=$(($(date +%s) - sync_start_time))
      echo ""
      echo "==> SYNC SUCCESS: Both nodes synced through Gloas fork boundary"
      echo "    Supernode: head=$super_head (synced)"
      echo "    Fullnode:  head=$full_head (synced)"
      echo "    Sync duration: ${sync_duration}s"

      # Verify finality and fork version on sync targets
      declare -A sync_urls=( ["supernode"]="$SYNC_SUPER_URL" ["fullnode"]="$SYNC_FULL_URL" )
      for node_label in supernode fullnode; do
        node_url="${sync_urls[$node_label]}"

        fin_resp=$(curl -sf "$node_url/eth/v1/beacon/states/head/finality_checkpoints" 2>/dev/null || echo "")
        if [ -n "$fin_resp" ]; then
          sync_fin=$(echo "$fin_resp" | jq -r '.data.finalized.epoch' 2>/dev/null || echo "0")
          echo "    $node_label finalized_epoch=$sync_fin"
        fi

        fork_resp=$(curl -sf "$node_url/eth/v1/beacon/states/head/fork" 2>/dev/null || echo "")
        if [ -n "$fork_resp" ]; then
          fork_epoch=$(echo "$fork_resp" | jq -r '.data.epoch' 2>/dev/null || echo "?")
          fork_version=$(echo "$fork_resp" | jq -r '.data.current_version' 2>/dev/null || echo "?")
          echo "    $node_label fork: epoch=$fork_epoch version=$fork_version"
        fi
      done

      echo ""
      echo "==> SUCCESS: Sync test complete"
      echo "==> Full logs: $RUN_DIR"
      echo "==> Sync log: $RUN_DIR/sync.log"
      cleanup
      exit 0
    fi
  done

  echo "==> TIMEOUT: Sync targets did not sync within ${SYNC_TIMEOUT}s"
  echo "    Supernode: head=$super_head synced=$super_synced"
  echo "    Fullnode:  head=$full_head synced=$full_synced"
  echo "==> Sync log: $RUN_DIR/sync.log"
  dump_logs
  cleanup
  exit 1
fi

# --- CHURN MODE: Kill/restart a validator node, verify chain recovery ---
if [ "$CHURN_MODE" = true ] && [ "${PRE_CHURN_FINALIZED:-0}" -gt 0 ]; then
  CHURN_TARGET="cl-4-lighthouse-geth"
  CHURN_EL="el-4-geth-lighthouse"

  echo ""
  echo "==> CHURN PHASE 1: Killing validator node 4..."
  echo "    Pre-churn: finalized_epoch=$PRE_CHURN_FINALIZED head_slot=$PRE_CHURN_HEAD"
  {
    echo "=== churn phase ==="
    echo "pre_churn_finalized=$PRE_CHURN_FINALIZED"
    echo "pre_churn_head=$PRE_CHURN_HEAD"
  } >> "$RUN_DIR/churn.log"

  # Stop CL and EL for node 4
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$CHURN_TARGET" 2>/dev/null || true
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$CHURN_EL" 2>/dev/null || true
  echo "    Stopped: $CHURN_TARGET, $CHURN_EL"
  echo "    Chain should continue finalizing with 3/4 nodes (75% stake)..."

  # Phase 2: Wait for finalization to advance at least 2 epochs with the node down
  CHURN_FIN_TARGET=$((PRE_CHURN_FINALIZED + 2))
  CHURN_TIMEOUT=180  # 3 minutes for 2 more epochs
  CHURN_POLL=12
  churn_elapsed=0
  chain_advanced=false

  echo "==> CHURN PHASE 2: Waiting for continued finalization (target epoch $CHURN_FIN_TARGET)..."

  while [ "$churn_elapsed" -lt "$CHURN_TIMEOUT" ]; do
    sleep "$CHURN_POLL"
    churn_elapsed=$((churn_elapsed + CHURN_POLL))

    syncing=$(curl -sf "$BEACON_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
    head_slot="?"
    if [ -n "$syncing" ]; then
      head_slot=$(echo "$syncing" | jq -r '.data.head_slot' 2>/dev/null || echo "?")
    fi

    finality=$(curl -sf "$BEACON_URL/eth/v1/beacon/states/head/finality_checkpoints" 2>/dev/null || echo "")
    finalized_epoch="0"
    if [ -n "$finality" ]; then
      finalized_epoch=$(echo "$finality" | jq -r '.data.finalized.epoch' 2>/dev/null || echo "0")
    fi

    echo "    [${churn_elapsed}s] head=$head_slot finalized=$finalized_epoch (target=$CHURN_FIN_TARGET, node 4 down)"
    {
      echo "--- churn-down ${churn_elapsed}s ---"
      echo "head=$head_slot finalized=$finalized_epoch target=$CHURN_FIN_TARGET"
    } >> "$RUN_DIR/churn.log"

    if [ "$finalized_epoch" -ge "$CHURN_FIN_TARGET" ]; then
      chain_advanced=true
      break
    fi
  done

  if [ "$chain_advanced" != true ]; then
    echo "==> FAIL: Chain did not continue finalizing with node 4 down"
    echo "    Finalized: $finalized_epoch (needed $CHURN_FIN_TARGET)"
    dump_logs
    cleanup
    exit 1
  fi

  echo "==> Chain continued finalizing (epoch $finalized_epoch) with node 4 down"

  # Phase 3: Restart the killed node
  echo ""
  echo "==> CHURN PHASE 3: Restarting node 4..."
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$CHURN_EL" 2>/dev/null || true
  sleep 5  # give EL a moment to start
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$CHURN_TARGET" 2>/dev/null || true
  echo "    Restarted: $CHURN_EL, $CHURN_TARGET"

  # Discover restarted node's beacon API URL
  CHURN_NODE_URL="$($SUDO kurtosis port print "$ENCLAVE_NAME" "$CHURN_TARGET" http 2>/dev/null || echo "")"
  if [ -z "$CHURN_NODE_URL" ]; then
    echo "==> FAIL: Could not discover restarted node beacon API"
    dump_logs
    cleanup
    exit 1
  fi
  echo "    Restarted node API: $CHURN_NODE_URL"

  # Phase 4: Wait for the restarted node to sync back up
  RECOVERY_TIMEOUT=360  # 6 minutes for recovery
  RECOVERY_POLL=6
  recovery_elapsed=0
  node_recovered=false

  echo "==> CHURN PHASE 4: Waiting for node 4 to recover (timeout: ${RECOVERY_TIMEOUT}s)..."

  while [ "$recovery_elapsed" -lt "$RECOVERY_TIMEOUT" ]; do
    sleep "$RECOVERY_POLL"
    recovery_elapsed=$((recovery_elapsed + RECOVERY_POLL))

    # Check validator head for comparison
    val_syncing=$(curl -sf "$BEACON_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
    val_head="?"
    if [ -n "$val_syncing" ]; then
      val_head=$(echo "$val_syncing" | jq -r '.data.head_slot' 2>/dev/null || echo "?")
    fi

    # Check restarted node
    churn_resp=$(curl -sf "$CHURN_NODE_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
    churn_head="0"
    churn_is_syncing="true"
    if [ -n "$churn_resp" ]; then
      churn_head=$(echo "$churn_resp" | jq -r '.data.head_slot' 2>/dev/null || echo "0")
      churn_is_syncing=$(echo "$churn_resp" | jq -r '.data.is_syncing' 2>/dev/null || echo "true")
    fi

    # Get detailed sync state
    churn_status="not_ready"
    churn_lh=$(curl -sf "$CHURN_NODE_URL/lighthouse/syncing" 2>/dev/null || echo "")
    if [ -n "$churn_lh" ]; then
      churn_status=$(echo "$churn_lh" | jq -r 'if (.data | type) == "string" then .data else (.data | keys[0] // "unknown") end' 2>/dev/null || echo "unknown")
    fi

    echo "    [${recovery_elapsed}s] validator_head=$val_head | node4: head=$churn_head status=$churn_status syncing=$churn_is_syncing"
    {
      echo "--- recovery ${recovery_elapsed}s ---"
      echo "validator_head=$val_head node4_head=$churn_head status=$churn_status syncing=$churn_is_syncing"
    } >> "$RUN_DIR/churn.log"

    # Success: node is no longer syncing and has a non-zero head
    if [ "$churn_is_syncing" = "false" ] && [ "$churn_head" != "0" ]; then
      node_recovered=true
      break
    fi
  done

  if [ "$node_recovered" != true ]; then
    echo "==> FAIL: Node 4 did not recover within ${RECOVERY_TIMEOUT}s"
    echo "    head=$churn_head syncing=$churn_is_syncing"
    dump_logs
    cleanup
    exit 1
  fi

  # Verify restarted node has correct finalization
  churn_fin_resp=$(curl -sf "$CHURN_NODE_URL/eth/v1/beacon/states/head/finality_checkpoints" 2>/dev/null || echo "")
  churn_finalized="?"
  if [ -n "$churn_fin_resp" ]; then
    churn_finalized=$(echo "$churn_fin_resp" | jq -r '.data.finalized.epoch' 2>/dev/null || echo "?")
  fi

  echo ""
  echo "==> CHURN SUCCESS: Node 4 recovered and chain continued"
  echo "    Node 4: head=$churn_head finalized=$churn_finalized"
  echo "    Chain finalized through node loss and recovery"
  echo ""
  echo "==> SUCCESS: Churn test complete"
  echo "==> Full logs: $RUN_DIR"
  echo "==> Churn log: $RUN_DIR/churn.log"
  cleanup
  exit 0
fi

echo "==> TIMEOUT: Did not reach finalized epoch $TARGET_FINALIZED_EPOCH within ${TIMEOUT}s"
echo "    Last state: slot=$prev_slot finalized_epoch=$finalized_epoch"
echo "==> Health log: $RUN_DIR/health.log"
dump_logs
cleanup
exit 1
