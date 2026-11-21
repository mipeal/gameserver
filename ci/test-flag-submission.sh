#!/usr/bin/env bash
set -euxo pipefail

# shellcheck source=/dev/null
. venv/bin/activate

# Configure database credentials for container setup
cp ./ci-config.yaml ./config.yaml
# schema bootstrap: alembic migrations were replaced by the sqlx baseline (ADR-005)
export PGPASSWORD=123456789
psql -h postgres -Uctf -d ctf -f rust/controlserver-rs/migrations/0001_baseline.sql

# Build the Rust gameserver bins (controlserver + checker workers + checker-exec)
# and the Rust ops scripts (reset-ctf), then run the Rust controlserver
cargo build --release --quiet \
    -p controlserver-rs --bin controlserver --bin checker_runner \
    -p checker-exec -p ctf-scripts-rs
export CONFIG="$(pwd)/config.yaml"
export CTF_PYTHON_DIR="$(pwd)"
export PATH="$(pwd)/target/release:$PATH"
./target/release/controlserver &

# Ensure the controlserver is fully started by building something in the mean time

# Build flag submission server
mkdir flag-submission-server/build
pushd flag-submission-server/build
cmake -DCMAKE_BUILD_TYPE=Release -DPostgreSQL_ADDITIONAL_VERSIONS=17 ..
make
popd

until curl --silent --output /dev/null http://127.0.0.1:5000/; do sleep 0.5; done

# Run some simple start/stop tests against the gameserver
./ci/ctf.py start
sleep 3
./ci/ctf.py pause
sleep 3
./ci/ctf.py start
sleep 3
./ci/vpn.sh close
sleep 3
./ci/vpn.sh open
sleep 3
./ci/ctf.py stop
echo "yes" | ./target/release/reset-ctf

# Start the gameserver and start the game
./ci/ctf.py start
# Test flag submitter
pushd flag-submission-server/build
./testsuite
./flag-submission-server > /tmp/submission.stdout 2>/tmp/submission.stderr &
# SUBMISSION_SERVER_PID=$!
sleep 1
./benchmark-newflags
./benchmark-oldflags
# Check response of flag submission
python3 ../scripts/submit_new_flags.py 10
killall ./flag-submission-server
echo "Submission Server Recorded Stdout:"
cat /tmp/submission.stdout
echo "Submission Server Recorded Stderr:"
cat /tmp/submission.stderr
popd
echo "yes" | ./target/release/reset-ctf
