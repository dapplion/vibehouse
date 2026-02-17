#!/usr/bin/env bash
set -euo pipefail

# Bounded devnet lifecycle for vibehouse.
# Build -> clean old enclave -> start -> poll assertoor -> teardown.
#
# Usage: scripts/kurtosis-run.sh [--no-build] [--no-teardown]
#
# Flags:
#   --no-build     Skip Docker image build (use existing vibehouse:local)
#   --no-teardown  Leave enclave running after completion (for inspection)
#
# Logs: each run writes to /tmp/kurtosis-runs/<RUN_ID>/ with separate files:
#   build.log       — cargo build + docker build output
#   kurtosis.log    — kurtosis run output (enclave startup)
#   assertoor.log   — assertoor polling output
#   dump/           — enclave dump on failure

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ENCLAVE_NAME="vibehouse-devnet"
KURTOSIS_CONFIG="$REPO_ROOT/kurtosis/vibehouse-epbs.yaml"
POLL_INTERVAL=15
TIMEOUT=300  # 5 minutes

DO_BUILD=true
DO_TEARDOWN=true

for arg in "$@"; do
  case "$arg" in
    --no-build)    DO_BUILD=false ;;
    --no-teardown) DO_TEARDOWN=false ;;
    *) echo "Unknown flag: $arg"; exit 1 ;;
  esac
done

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
if ! kurtosis run github.com/ethpandaops/ethereum-package --enclave "$ENCLAVE_NAME" --args-file "$KURTOSIS_CONFIG" > "$RUN_DIR/kurtosis.log" 2>&1; then
  echo "==> FAIL: kurtosis run failed. See $RUN_DIR/kurtosis.log"
  echo "--- last 30 lines ---"
  tail -30 "$RUN_DIR/kurtosis.log"
  cleanup
  exit 1
fi
echo "==> Devnet started. Services:"
tail -30 "$RUN_DIR/kurtosis.log" | grep -E '(RUNNING|STOPPED|Name)' || true

# Step 4: Discover assertoor port
echo "==> Discovering assertoor endpoint..."
ASSERTOOR_URL="$(kurtosis port print "$ENCLAVE_NAME" assertoor http)"
echo "    Assertoor: $ASSERTOOR_URL"

# Step 5: Poll assertoor for test results
echo "==> Polling assertoor (timeout: ${TIMEOUT}s, interval: ${POLL_INTERVAL}s)..."
elapsed=0

{
while [ "$elapsed" -lt "$TIMEOUT" ]; do
  sleep "$POLL_INTERVAL"
  elapsed=$((elapsed + POLL_INTERVAL))

  # Fetch test status from assertoor API
  response=$(curl -sf "$ASSERTOOR_URL/api/v1/test_status" 2>/dev/null || echo "")

  if [ -z "$response" ]; then
    echo "    [${elapsed}s] Assertoor not ready yet..."
    continue
  fi

  # Log full response
  echo "--- ${elapsed}s ---" >> "$RUN_DIR/assertoor.log"
  echo "$response" | jq . >> "$RUN_DIR/assertoor.log" 2>/dev/null || echo "$response" >> "$RUN_DIR/assertoor.log"

  # Check overall status
  total=$(echo "$response" | jq -r '.data.tests | length' 2>/dev/null || echo "0")
  passing=$(echo "$response" | jq -r '[.data.tests[] | select(.status == "success")] | length' 2>/dev/null || echo "0")
  failing=$(echo "$response" | jq -r '[.data.tests[] | select(.status == "failure")] | length' 2>/dev/null || echo "0")
  running=$(echo "$response" | jq -r '[.data.tests[] | select(.status == "running" or .status == "pending")] | length' 2>/dev/null || echo "0")

  echo "    [${elapsed}s] Tests: $passing passed, $failing failed, $running running (total: $total)"

  if [ "$failing" -gt 0 ]; then
    echo "==> FAIL: Assertoor reports test failures"
    echo "$response" | jq '.data.tests[] | select(.status == "failure") | {name, status, message}' 2>/dev/null || true
    dump_logs
    cleanup
    exit 1
  fi

  if [ "$total" -gt 0 ] && [ "$running" -eq 0 ] && [ "$passing" -eq "$total" ]; then
    echo "==> SUCCESS: All $total assertoor tests passed!"
    echo "==> Full logs: $RUN_DIR"
    cleanup
    exit 0
  fi
done
}

echo "==> TIMEOUT: Assertoor did not report all-pass within ${TIMEOUT}s"
echo "==> Assertoor poll log: $RUN_DIR/assertoor.log"
dump_logs
cleanup
exit 1
