#!/usr/bin/env bash
set -euo pipefail

# Fast Docker image build for vibehouse dev.
# Builds the lighthouse binary on the host (incremental, uses cargo cache)
# then packages it into a minimal Docker image via Dockerfile.dev.
#
# Usage: scripts/build-docker.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"

echo "==> Building lighthouse (release) on host..."
cargo build --release --features spec-minimal

echo "==> Copying binary to bin/..."
mkdir -p bin
cp target/release/lighthouse bin/lighthouse

echo "==> Building Docker image vibehouse:local..."
# Use sudo if current user can't access docker socket directly
DOCKER_CMD="docker"
if ! docker info >/dev/null 2>&1; then
  DOCKER_CMD="sudo docker"
fi
$DOCKER_CMD build -f Dockerfile.dev -t vibehouse:local .

echo "==> Done. Image: vibehouse:local"
