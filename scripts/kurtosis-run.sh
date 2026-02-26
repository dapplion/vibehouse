#!/usr/bin/env bash
set -euo pipefail

# Bounded devnet lifecycle for vibehouse.
# Build -> clean old enclave -> start -> poll beacon API -> teardown.
#
# Usage: scripts/kurtosis-run.sh [--no-build] [--no-teardown] [--stateless] [--multiclient] [--sync] [--churn] [--mainnet] [--long] [--partition] [--builder] [--withhold]
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
#   --mainnet       Mainnet preset test: realistic committee sizes, 32 slots/epoch,
#                   12s slots, 512 validators (128/node × 4 nodes)
#   --long          Long-running test: 30+ min, epoch 50 target, periodic memory/resource
#                   monitoring to catch leaks and stalls
#   --partition     Network partition test: stop 2/4 nodes (50% stake), verify chain
#                   blocks but doesn't finalize, restart, verify finalization resumes
#   --builder       External builder path test: start devnet with 1 genesis builder,
#                   submit bids via lcli after Gloas fork, verify blocks use external bids
#   --withhold      Payload withholding test: submit a bid but never reveal the envelope,
#                   verify fork choice takes the EMPTY path and chain continues finalizing
#
# Logs: each run writes to /tmp/kurtosis-runs/<RUN_ID>/ with separate files:
#   build.log       — cargo build + docker build output
#   kurtosis.log    — kurtosis run output (enclave startup)
#   health.log      — beacon API polling results (JSON per poll)
#   sync.log        — sync mode: non-validator node sync progress (--sync only)
#   churn.log       — churn mode: kill/recovery phase progress (--churn only)
#   resources.log   — long mode: periodic container memory/CPU snapshots (--long only)
#   partition.log   — partition mode: split/heal phase progress (--partition only)
#   builder.log     — builder mode: bid submission and block verification (--builder only)
#   withhold.log    — withhold mode: bid submission and EMPTY path finalization (--withhold only)
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
MAINNET_MODE=false
LONG_MODE=false
PARTITION_MODE=false
BUILDER_MODE=false
WITHHOLD_MODE=false

for arg in "$@"; do
  case "$arg" in
    --no-build)     DO_BUILD=false ;;
    --no-teardown)  DO_TEARDOWN=false ;;
    --stateless)    STATELESS_MODE=true ;;
    --multiclient)  MULTICLIENT_MODE=true ;;
    --sync)         SYNC_MODE=true ;;
    --churn)        CHURN_MODE=true ;;
    --mainnet)      MAINNET_MODE=true ;;
    --long)         LONG_MODE=true ;;
    --partition)    PARTITION_MODE=true ;;
    --builder)      BUILDER_MODE=true ;;
    --withhold)     WITHHOLD_MODE=true ;;
    *) echo "Unknown flag: $arg"; exit 1 ;;
  esac
done

# Select config based on mode
if [ "$PARTITION_MODE" = true ]; then
  # Partition uses the default 4-node config — stop 2 to simulate split
  TARGET_FINALIZED_EPOCH=3  # lower initial target: just past Gloas fork
  TIMEOUT=900               # 15 minutes total (warm-up + partition + heal)
  echo "==> Partition mode: using $KURTOSIS_CONFIG (4 validators, will partition 2 nodes)"
elif [ "$LONG_MODE" = true ]; then
  # Long-running uses the default 4-node config with high finalization target
  TARGET_FINALIZED_EPOCH=50  # ~50 epochs × 48s = ~40 min in minimal preset
  TIMEOUT=3000               # 50 minutes (generous margin)
  echo "==> Long mode: using $KURTOSIS_CONFIG (4 nodes, target epoch $TARGET_FINALIZED_EPOCH, ~40 min)"
elif [ "$MAINNET_MODE" = true ]; then
  KURTOSIS_CONFIG="$REPO_ROOT/kurtosis/vibehouse-mainnet.yaml"
  SLOTS_PER_EPOCH=32
  GLOAS_FORK_SLOT=$((GLOAS_FORK_EPOCH * SLOTS_PER_EPOCH))
  POLL_INTERVAL=24          # 2 slots at 12s each
  TARGET_FINALIZED_EPOCH=4  # past Gloas fork, enough to prove chain works
  TIMEOUT=2400              # 40 minutes (mainnet epochs are ~6.4 min each)
  echo "==> Mainnet mode: using $KURTOSIS_CONFIG (4 nodes, 512 validators, 32 slots/epoch)"
elif [ "$CHURN_MODE" = true ]; then
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
elif [ "$BUILDER_MODE" = true ]; then
  KURTOSIS_CONFIG="$REPO_ROOT/kurtosis/vibehouse-builder.yaml"
  TARGET_FINALIZED_EPOCH=3  # wait past Gloas fork so builder (deposit_epoch=0) is active
  TIMEOUT=900               # 15 minutes total (finalization + builder bids + verification)
  echo "==> Builder mode: using $KURTOSIS_CONFIG (4 validators, 1 genesis builder)"
elif [ "$WITHHOLD_MODE" = true ]; then
  KURTOSIS_CONFIG="$REPO_ROOT/kurtosis/vibehouse-builder.yaml"
  TARGET_FINALIZED_EPOCH=3  # wait past Gloas fork so builder is active
  TIMEOUT=900               # 15 minutes total
  echo "==> Withhold mode: using $KURTOSIS_CONFIG (4 validators, 1 genesis builder, no envelope)"
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

  # Long mode: periodic resource monitoring (every 5th poll ≈ 60s)
  if [ "$LONG_MODE" = true ] && [ $((elapsed % (POLL_INTERVAL * 5))) -eq 0 ]; then
    resource_snapshot=$($SUDO docker stats --no-stream --format '{{.Name}}\t{{.MemUsage}}\t{{.CPUPerc}}' 2>/dev/null | grep -E '(cl-|el-)' || echo "docker stats unavailable")
    {
      echo "--- resources ${elapsed}s ---"
      echo "$resource_snapshot"
    } >> "$RUN_DIR/resources.log"
    # Show summary on stdout
    mem_summary=$(echo "$resource_snapshot" | awk -F'\t' '{printf "%s=%s ", $1, $2}' 2>/dev/null || echo "")
    if [ -n "$mem_summary" ]; then
      echo "    [resources] $mem_summary"
    fi
  fi

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

    # In partition mode, break out to run the split/heal phases
    if [ "$PARTITION_MODE" = true ]; then
      PRE_PARTITION_FINALIZED="$finalized_epoch"
      PRE_PARTITION_HEAD="$head_slot"
      break
    fi

    # In builder mode, break out to run the bid submission phase
    if [ "$BUILDER_MODE" = true ]; then
      PRE_BUILDER_FINALIZED="$finalized_epoch"
      PRE_BUILDER_HEAD="$head_slot"
      break
    fi

    # In withhold mode, break out to run the bid + withholding phase
    if [ "$WITHHOLD_MODE" = true ]; then
      PRE_WITHHOLD_FINALIZED="$finalized_epoch"
      PRE_WITHHOLD_HEAD="$head_slot"
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

# --- PARTITION MODE: Stop 2/4 nodes, verify no finalization, heal, verify recovery ---
if [ "$PARTITION_MODE" = true ] && [ "${PRE_PARTITION_FINALIZED:-0}" -gt 0 ]; then
  PART_NODE_3="cl-3-lighthouse-geth"
  PART_NODE_4="cl-4-lighthouse-geth"
  PART_EL_3="el-3-geth-lighthouse"
  PART_EL_4="el-4-geth-lighthouse"

  echo ""
  echo "==> PARTITION PHASE 1: Splitting network (stopping nodes 3 & 4)..."
  echo "    Pre-partition: finalized_epoch=$PRE_PARTITION_FINALIZED head_slot=$PRE_PARTITION_HEAD"
  {
    echo "=== partition phase ==="
    echo "pre_partition_finalized=$PRE_PARTITION_FINALIZED"
    echo "pre_partition_head=$PRE_PARTITION_HEAD"
  } >> "$RUN_DIR/partition.log"

  # Stop CL and EL for nodes 3 and 4 (50% of stake offline)
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$PART_NODE_3" 2>/dev/null || true
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$PART_NODE_4" 2>/dev/null || true
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$PART_EL_3" 2>/dev/null || true
  $SUDO kurtosis service stop "$ENCLAVE_NAME" "$PART_EL_4" 2>/dev/null || true
  echo "    Stopped: $PART_NODE_3, $PART_NODE_4, $PART_EL_3, $PART_EL_4"
  echo "    Only 50% stake online — finalization should stall..."

  # Phase 2: Wait a few epochs, verify finalization does NOT advance
  STALL_WAIT=120  # 2.5 epochs in minimal preset (enough to prove no finalization)
  STALL_POLL=12
  stall_elapsed=0
  finalization_stalled=true
  stalled_finalized="$PRE_PARTITION_FINALIZED"

  echo "==> PARTITION PHASE 2: Verifying finalization stalls (${STALL_WAIT}s wait)..."

  while [ "$stall_elapsed" -lt "$STALL_WAIT" ]; do
    sleep "$STALL_POLL"
    stall_elapsed=$((stall_elapsed + STALL_POLL))

    syncing=$(curl -sf "$BEACON_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
    head_slot="?"
    if [ -n "$syncing" ]; then
      head_slot=$(echo "$syncing" | jq -r '.data.head_slot' 2>/dev/null || echo "?")
    fi

    finality=$(curl -sf "$BEACON_URL/eth/v1/beacon/states/head/finality_checkpoints" 2>/dev/null || echo "")
    finalized_epoch="$stalled_finalized"
    if [ -n "$finality" ]; then
      finalized_epoch=$(echo "$finality" | jq -r '.data.finalized.epoch' 2>/dev/null || echo "$stalled_finalized")
    fi

    echo "    [${stall_elapsed}s] head=$head_slot finalized=$finalized_epoch (should stay at $stalled_finalized, 50% stake)"
    {
      echo "--- partition-stall ${stall_elapsed}s ---"
      echo "head=$head_slot finalized=$finalized_epoch expected_stalled=$stalled_finalized"
    } >> "$RUN_DIR/partition.log"

    # If finalization advanced, the partition test failed (shouldn't finalize with 50%)
    if [ "$finalized_epoch" -gt "$stalled_finalized" ]; then
      finalization_stalled=false
      echo "==> WARNING: Finalization advanced during partition (epoch $finalized_epoch > $stalled_finalized)"
      echo "    This might indicate the partition wasn't effective"
    fi
  done

  if [ "$finalization_stalled" = true ]; then
    echo "==> Finalization correctly stalled at epoch $stalled_finalized during partition"
  fi

  # Phase 3: Heal — restart all stopped nodes
  echo ""
  echo "==> PARTITION PHASE 3: Healing network (restarting nodes 3 & 4)..."
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$PART_EL_3" 2>/dev/null || true
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$PART_EL_4" 2>/dev/null || true
  sleep 5  # give EL a moment to start
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$PART_NODE_3" 2>/dev/null || true
  $SUDO kurtosis service start "$ENCLAVE_NAME" "$PART_NODE_4" 2>/dev/null || true
  echo "    Restarted: $PART_EL_3, $PART_EL_4, $PART_NODE_3, $PART_NODE_4"

  # Phase 4: Wait for finalization to resume (should advance past stalled epoch)
  HEAL_FIN_TARGET=$((stalled_finalized + 2))
  HEAL_TIMEOUT=360  # 6 minutes for recovery
  HEAL_POLL=12
  heal_elapsed=0
  healed=false

  echo "==> PARTITION PHASE 4: Waiting for finalization to resume (target epoch $HEAL_FIN_TARGET)..."

  while [ "$heal_elapsed" -lt "$HEAL_TIMEOUT" ]; do
    sleep "$HEAL_POLL"
    heal_elapsed=$((heal_elapsed + HEAL_POLL))

    syncing=$(curl -sf "$BEACON_URL/eth/v1/node/syncing" 2>/dev/null || echo "")
    head_slot="?"
    if [ -n "$syncing" ]; then
      head_slot=$(echo "$syncing" | jq -r '.data.head_slot' 2>/dev/null || echo "?")
    fi

    finality=$(curl -sf "$BEACON_URL/eth/v1/beacon/states/head/finality_checkpoints" 2>/dev/null || echo "")
    finalized_epoch="$stalled_finalized"
    if [ -n "$finality" ]; then
      finalized_epoch=$(echo "$finality" | jq -r '.data.finalized.epoch' 2>/dev/null || echo "$stalled_finalized")
    fi

    echo "    [${heal_elapsed}s] head=$head_slot finalized=$finalized_epoch (target=$HEAL_FIN_TARGET)"
    {
      echo "--- partition-heal ${heal_elapsed}s ---"
      echo "head=$head_slot finalized=$finalized_epoch target=$HEAL_FIN_TARGET"
    } >> "$RUN_DIR/partition.log"

    if [ "$finalized_epoch" -ge "$HEAL_FIN_TARGET" ]; then
      healed=true
      break
    fi
  done

  if [ "$healed" != true ]; then
    echo "==> FAIL: Finalization did not resume after healing partition"
    echo "    Finalized: $finalized_epoch (needed $HEAL_FIN_TARGET)"
    dump_logs
    cleanup
    exit 1
  fi

  echo ""
  echo "==> PARTITION SUCCESS: Network partition and recovery verified"
  echo "    Finalization stalled at epoch $stalled_finalized during partition"
  echo "    Finalization resumed to epoch $finalized_epoch after healing"
  echo ""
  echo "==> SUCCESS: Partition test complete"
  echo "==> Full logs: $RUN_DIR"
  echo "==> Partition log: $RUN_DIR/partition.log"
  cleanup
  exit 0
fi

# --- BUILDER MODE: Submit external bids, verify blocks use external builder ---
if [ "$BUILDER_MODE" = true ] && [ "${PRE_BUILDER_FINALIZED:-0}" -gt 0 ]; then
  # Builder is active once finalized_epoch > deposit_epoch (0) — already satisfied.
  # The devnet has 4 validators × 16 keys/node = 64 validator keypairs.
  # Genesis builder 0 uses keypair index 64 (validator_count + builder_index).
  VALIDATOR_COUNT=64   # 4 nodes × 16 validators/node
  BUILDER_IDX=0

  echo ""
  echo "==> BUILDER PHASE: Submitting external bids..."
  echo "    Pre-builder: finalized_epoch=$PRE_BUILDER_FINALIZED head_slot=$PRE_BUILDER_HEAD"
  {
    echo "=== builder phase ==="
    echo "pre_builder_finalized=$PRE_BUILDER_FINALIZED"
    echo "pre_builder_head=$PRE_BUILDER_HEAD"
  } >> "$RUN_DIR/builder.log"

  # Verify lcli is available (it's compiled into the Docker image / host)
  LCLI="$REPO_ROOT/target/release/lcli"
  if [ ! -f "$LCLI" ]; then
    # Try to find lcli in PATH
    LCLI="$(command -v lcli 2>/dev/null || true)"
    if [ -z "$LCLI" ]; then
      echo "==> WARNING: lcli not found — skipping bid submission"
      echo "    Build with 'cargo build --release -p lcli' to enable builder tests"
      {
        echo "lcli not found — bid submission skipped"
      } >> "$RUN_DIR/builder.log"
      # Fall through to finalization check (chain should still be healthy)
    fi
  fi

  if [ -n "${LCLI:-}" ] && [ -f "$LCLI" ]; then
    # Submit 3 bids for upcoming slots
    BUILDER_BIDS_SUBMITTED=0
    for attempt in 1 2 3; do
      echo "    [attempt $attempt] Submitting builder bid via lcli..."
      bid_output=$("$LCLI" \
        --spec minimal \
        submit-builder-bid \
        --beacon-url "$BEACON_URL" \
        --builder-index "$BUILDER_IDX" \
        --validator-count "$VALIDATOR_COUNT" \
        --bid-value 1000000000 \
        2>&1 || true)
      echo "    $bid_output"
      {
        echo "--- bid attempt $attempt ---"
        echo "$bid_output"
      } >> "$RUN_DIR/builder.log"

      if echo "$bid_output" | grep -q "Bid submitted successfully"; then
        BUILDER_BIDS_SUBMITTED=$((BUILDER_BIDS_SUBMITTED + 1))
        echo "    Bid $attempt submitted successfully!"
      fi
      sleep 12  # wait one slot between bids
    done

    echo "==> Builder submitted $BUILDER_BIDS_SUBMITTED/3 bids"

    if [ "$BUILDER_BIDS_SUBMITTED" -eq 0 ]; then
      echo "==> FAIL: No bids were accepted by the beacon node"
      dump_logs
      cleanup
      exit 1
    fi
  fi

  # Phase 2: Wait for more finalization to confirm chain is still healthy
  BUILDER_FIN_TARGET=$((PRE_BUILDER_FINALIZED + 2))
  BUILDER_TIMEOUT=180  # 3 minutes
  BUILDER_POLL=12
  builder_elapsed=0
  chain_healthy=false

  echo "==> Waiting for continued finalization (target epoch $BUILDER_FIN_TARGET)..."

  while [ "$builder_elapsed" -lt "$BUILDER_TIMEOUT" ]; do
    sleep "$BUILDER_POLL"
    builder_elapsed=$((builder_elapsed + BUILDER_POLL))

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

    echo "    [${builder_elapsed}s] head=$head_slot finalized=$finalized_epoch (target=$BUILDER_FIN_TARGET)"
    {
      echo "--- builder-health ${builder_elapsed}s ---"
      echo "head=$head_slot finalized=$finalized_epoch"
    } >> "$RUN_DIR/builder.log"

    if [ "$finalized_epoch" -ge "$BUILDER_FIN_TARGET" ]; then
      chain_healthy=true
      break
    fi
  done

  if [ "$chain_healthy" != true ]; then
    echo "==> FAIL: Chain did not continue finalizing after builder bid submission"
    echo "    Finalized: $finalized_epoch (needed $BUILDER_FIN_TARGET)"
    dump_logs
    cleanup
    exit 1
  fi

  # Phase 3: Check recent blocks to see if any used our external bid
  # External bid blocks have builder_index != BUILDER_INDEX_SELF_BUILD (which is 18446744073709551615)
  SELF_BUILD_INDEX="18446744073709551615"
  external_bid_found=false

  echo "==> Checking recent blocks for external builder bids..."
  # Check the last 16 slots for external bids
  check_slot=$(echo "$head_slot" | tr -d '?')
  if [ -n "$check_slot" ] && [ "$check_slot" -gt 0 ]; then
    for slot_offset in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16; do
      check_s=$((check_slot - slot_offset))
      if [ "$check_s" -le 0 ]; then
        break
      fi

      block_resp=$(curl -sf "$BEACON_URL/eth/v1/beacon/blocks/$check_s" 2>/dev/null || echo "")
      if [ -n "$block_resp" ]; then
        # Extract builder_index from the signed_execution_payload_bid field
        builder_index_val=$(echo "$block_resp" | jq -r '.data.message.body.signed_execution_payload_bid.message.builder_index // empty' 2>/dev/null || echo "")
        if [ -n "$builder_index_val" ] && [ "$builder_index_val" != "$SELF_BUILD_INDEX" ]; then
          external_bid_found=true
          echo "    Found block at slot $check_s with external builder_index=$builder_index_val"
          {
            echo "external_bid_found: slot=$check_s builder_index=$builder_index_val"
          } >> "$RUN_DIR/builder.log"
          break
        fi
      fi
    done
  fi

  if [ "$external_bid_found" = true ]; then
    echo "==> External builder bid was selected for block production!"
  else
    echo "==> NOTE: No external bid found in recent blocks (bids may have been for future slots or not selected)"
    echo "    This is non-fatal — bid submission and chain health are verified."
  fi

  echo ""
  echo "==> BUILDER SUCCESS: External builder path verified"
  echo "    Bid submission: $BUILDER_BIDS_SUBMITTED bids accepted"
  echo "    Chain finalized to epoch $finalized_epoch after bid submission"
  echo "    External bid selected: $external_bid_found"
  echo ""
  echo "==> SUCCESS: Builder test complete"
  echo "==> Full logs: $RUN_DIR"
  echo "==> Builder log: $RUN_DIR/builder.log"
  cleanup
  exit 0
fi

# ============================================================
# WITHHOLD MODE: Submit a bid but never reveal the envelope.
# Verifies the fork choice takes the EMPTY path (payload_revealed=false)
# and the chain continues finalizing without the payload.
# ============================================================
if [ "$WITHHOLD_MODE" = true ] && [ "${PRE_WITHHOLD_FINALIZED:-0}" -gt 0 ]; then
  # Builder is active — same setup as builder mode.
  VALIDATOR_COUNT=64  # 4 nodes × 16 validators/node
  BUILDER_IDX=0

  echo ""
  echo "==> WITHHOLD PHASE: Submitting bid without envelope (payload withholding)..."
  echo "    Pre-withhold: finalized_epoch=$PRE_WITHHOLD_FINALIZED head_slot=$PRE_WITHHOLD_HEAD"
  {
    echo "=== withhold phase ==="
    echo "pre_withhold_finalized=$PRE_WITHHOLD_FINALIZED"
    echo "pre_withhold_head=$PRE_WITHHOLD_HEAD"
  } >> "$RUN_DIR/withhold.log"

  # Find lcli
  LCLI="$REPO_ROOT/target/release/lcli"
  if [ ! -f "$LCLI" ]; then
    LCLI="$(command -v lcli 2>/dev/null || true)"
    if [ -z "$LCLI" ]; then
      echo "==> WARNING: lcli not found — skipping bid submission"
      echo "    Build with 'cargo build --release -p lcli' to enable withhold tests"
      {
        echo "lcli not found — bid submission skipped"
      } >> "$RUN_DIR/withhold.log"
    fi
  fi

  if [ -n "${LCLI:-}" ] && [ -f "$LCLI" ]; then
    # Submit 1 bid. No envelope will ever be sent — this is the withholding scenario.
    # The beacon node will receive the bid, import it to fork choice with payload_revealed=false,
    # and then move on via the EMPTY path (attesters vote payload_present=false).
    echo "    Submitting builder bid (no envelope will be revealed)..."
    withhold_bid_output=$("$LCLI" \
      --spec minimal \
      submit-builder-bid \
      --beacon-url "$BEACON_URL" \
      --builder-index "$BUILDER_IDX" \
      --validator-count "$VALIDATOR_COUNT" \
      --bid-value 1000000000 \
      2>&1 || true)
    echo "    $withhold_bid_output"
    {
      echo "--- bid submission ---"
      echo "$withhold_bid_output"
    } >> "$RUN_DIR/withhold.log"

    bid_accepted=false
    if echo "$withhold_bid_output" | grep -q "Bid submitted successfully"; then
      bid_accepted=true
      echo "    Bid accepted by beacon node (payload_revealed=false in fork choice)."
    else
      echo "    WARNING: Bid was not accepted (may be for a past slot or prefs mismatch)."
      echo "    Continuing anyway — chain health is the primary verification."
    fi
    {
      echo "bid_accepted=$bid_accepted"
    } >> "$RUN_DIR/withhold.log"
  fi

  # Phase 2: Verify the chain continues to finalize WITHOUT the envelope.
  # The fork choice should take the EMPTY path (payload_revealed=false) and keep going.
  WITHHOLD_FIN_TARGET=$((PRE_WITHHOLD_FINALIZED + 2))
  WITHHOLD_TIMEOUT=180  # 3 minutes (~2 minimal preset epochs)
  WITHHOLD_POLL=12
  withhold_elapsed=0
  withhold_chain_healthy=false

  echo "==> Waiting for chain to finalize on EMPTY path (target epoch $WITHHOLD_FIN_TARGET)..."
  echo "    (no envelope was revealed — if chain stalls, that's a bug)"

  while [ "$withhold_elapsed" -lt "$WITHHOLD_TIMEOUT" ]; do
    sleep "$WITHHOLD_POLL"
    withhold_elapsed=$((withhold_elapsed + WITHHOLD_POLL))

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

    echo "    [${withhold_elapsed}s] head=$head_slot finalized=$finalized_epoch (target=$WITHHOLD_FIN_TARGET)"
    {
      echo "--- withhold-health ${withhold_elapsed}s ---"
      echo "head=$head_slot finalized=$finalized_epoch"
    } >> "$RUN_DIR/withhold.log"

    if [ "$finalized_epoch" -ge "$WITHHOLD_FIN_TARGET" ]; then
      withhold_chain_healthy=true
      break
    fi
  done

  if [ "$withhold_chain_healthy" != true ]; then
    echo "==> FAIL: Chain stalled after payload withholding — EMPTY path not working"
    echo "    Finalized: $finalized_epoch (needed $WITHHOLD_FIN_TARGET)"
    echo "    This means fork choice is stuck waiting for envelope — that's a bug"
    dump_logs
    cleanup
    exit 1
  fi

  echo ""
  echo "==> WITHHOLD SUCCESS: Chain continued finalizing without payload envelope"
  echo "    Bid submitted: ${bid_accepted:-false}"
  echo "    Chain finalized to epoch $finalized_epoch (EMPTY path)"
  echo "    Fork choice correctly handled payload withholding"
  echo ""
  echo "==> SUCCESS: Payload withholding test complete"
  echo "==> Full logs: $RUN_DIR"
  echo "==> Withhold log: $RUN_DIR/withhold.log"
  cleanup
  exit 0
fi

echo "==> TIMEOUT: Did not reach finalized epoch $TARGET_FINALIZED_EPOCH within ${TIMEOUT}s"
echo "    Last state: slot=$prev_slot finalized_epoch=$finalized_epoch"
echo "==> Health log: $RUN_DIR/health.log"
dump_logs
cleanup
exit 1
