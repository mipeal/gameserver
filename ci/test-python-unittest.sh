#!/usr/bin/env bash
set -euxo pipefail

# Run static checker
make check

# Configure database credentials for container setup
sed 's/"database": "ctf"/"database": "ctf_unittest"/' ./ci-config.yaml > ./config.test.yaml
export PGPASSWORD=123456789
echo 'CREATE DATABASE ctf_unittest;' | psql -Uctf -h postgres ctf

make test
