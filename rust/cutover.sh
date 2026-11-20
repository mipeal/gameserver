#!/usr/bin/env bash
# Switch the gameserver docker stack between the Python original and the Rust port.
# Both stacks share the project name "gameserver" (named volumes: postgres, redis,
# public/internal-scoreboard), so switching preserves all state.
#
# Usage: cutover.sh build | rust | python
#   build  - build the wasm SPA + multi-arch docker image (linux/amd64,linux/arm64)
#   rust   - stop the Python stack, start the Rust stack
#   python - stop the Rust stack, start the Python stack (rollback)
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

case "${1:-}" in
  build)
    build
    ;;
  rust)
    docker compose -p gameserver -f docker-compose.yml down --remove-orphans || true
    docker compose -p gameserver -f "$COMPOSE_RUST" up -d --remove-orphans
    ;;
  python)
    docker compose -p gameserver -f "$COMPOSE_RUST" down --remove-orphans || true
    docker compose -p gameserver -f docker-compose.yml up -d
    ;;
  *)
    echo "usage: $0 build|rust|python" >&2
    exit 1
    ;;
esac
