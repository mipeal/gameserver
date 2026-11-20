#!/usr/bin/env bash
# Switch the gameserver docker stack between the Python original and the Rust port.
# Both stacks share the project name "gameserver" (named volumes: postgres, redis,
# public/internal-scoreboard), so switching preserves all state.
#
# Usage: cutover.sh build | rust | python
#   build  - build the wasm SPA + rust binaries + docker image
#   rust   - stop the Python stack, start the Rust stack
#   python - stop the Rust stack, start the Python stack (rollback)
#
# NOTE: `build` must run on the target host (linux/amd64) — the rust binaries
# are copied into the image as host builds, not cross-compiled in docker.
set -euo pipefail
cd "$(dirname "$0")/.."

COMPOSE_RUST=rust/docker-compose.rust.yml
BIN_DIR=rust/controlserver-rs/bin
IMAGE="${IMAGE_REGISTRY:-localhost}/gameserver-rust"

build() {
  echo "== building scoreboard SPA (trunk) =="
  (cd rust/scoreboard-rs && trunk build --release)

  echo "== building rust binaries (controlserver, checker_runner, scoreboard) =="
  (cd rust/controlserver-rs && cargo build --release --bins)
  mkdir -p "$BIN_DIR"
  cp rust/controlserver-rs/target/release/controlserver \
     rust/controlserver-rs/target/release/checker_runner \
     rust/controlserver-rs/target/release/scoreboard \
     "$BIN_DIR"/

  echo "== building docker image $IMAGE =="
  docker build -f rust/controlserver-rs/Dockerfile.rust -t "$IMAGE" .
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
