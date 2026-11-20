#!/usr/bin/env bash
# Build and run the gameserver Rust stack (the Python stack was removed after
# the cutover; the old docker-compose.yml/Dockerfile are recoverable from git
# history).
#
# Usage: cutover.sh build | rust
#   build  - build the wasm SPA + multi-arch docker image (linux/amd64,linux/arm64)
#   rust   - start the Rust stack
#
# Image build: `docker buildx build --platform linux/amd64,linux/arm64` — the rust
# bins are compiled inside docker per target platform, so no cross toolchain is
# needed on the host. A multi-platform image can only be pushed to a registry
# (`--push`); for a purely local image set PLATFORMS to the host platform.
set -euo pipefail
cd "$(dirname "$0")/.."

COMPOSE_RUST=rust/docker-compose.rust.yml
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
IMAGE="${IMAGE_REGISTRY:-localhost}/gameserver-rust"

build() {
  echo "== building scoreboard SPA (trunk) =="
  (cd rust/scoreboard-rs && trunk build --release)

  echo "== building docker image $IMAGE ($PLATFORMS) =="
  if [[ "$PLATFORMS" == *,* ]]; then
    # multi-platform output requires a registry; localhost images are single-arch
    docker buildx build --platform "$PLATFORMS" \
      -f rust/controlserver-rs/Dockerfile.rust -t "$IMAGE" --push .
  else
    docker buildx build --platform "$PLATFORMS" \
      -f rust/controlserver-rs/Dockerfile.rust -t "$IMAGE" --load .
  fi
}

# Local port overrides (host 5000/6379 busy on some machines) — explicit -f
# suppresses auto-loading, so append it when present.
OVERRIDE=""
[ -f rust/docker-compose.override.yml ] && OVERRIDE="-f rust/docker-compose.override.yml"

case "${1:-}" in
  build)
    build
    ;;
  rust)
    docker compose -p gameserver -f "$COMPOSE_RUST" $OVERRIDE up -d --remove-orphans
    ;;
  *)
    echo "usage: $0 build|rust" >&2
    exit 1
    ;;
esac
