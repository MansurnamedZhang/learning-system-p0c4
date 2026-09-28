#!/bin/sh
set -eu
. /app/deploy/c3-env.sh
# All logs belong to this new volume; failures retain their original exits.
trap 'for c3_log in /evidence/*.json /evidence/*.log /evidence/*.exit; do if [ -f "$c3_log" ]; then sha256sum "$c3_log"; fi; done > /evidence/results.sha256' 0
run_gate() {
    c3_gate_name=$1
    shift
    printf '%s\n' "$*" > "/evidence/$c3_gate_name.command.log"
    set +e
    "$@" > "/evidence/$c3_gate_name.stdout.log" 2> "/evidence/$c3_gate_name.stderr.log"
    c3_gate_status=$?
    set -e
    printf '%s\n' "$c3_gate_status" > "/evidence/$c3_gate_name.exit"
    cat "/evidence/$c3_gate_name.stdout.log"
    cat "/evidence/$c3_gate_name.stderr.log" >&2
    return "$c3_gate_status"
}
# Refuse every pre-existing schema before any bootstrap/migration.
run_gate empty-databases cargo run --offline --locked -p learning-worker --example c3_acceptance -- empty || exit $?
cp /evidence/empty-databases.stdout.log /evidence/empty-databases.json
export FIXTURE_EVIDENCE_DIR=/evidence
. /app/deploy/bootstrap-fixtures.sh
run_gate workspace cargo test --offline --locked --workspace -- --test-threads=1 || exit $?
run_gate legacy-upgrades cargo run --offline --locked -p learning-worker --example c3_acceptance -- legacy-upgrades || exit $?
cp /evidence/legacy-upgrades.stdout.log /evidence/legacy-upgrades.json
run_gate fmt cargo fmt --all -- --check || exit $?
run_gate clippy cargo clippy --offline --locked --workspace --all-targets -- -D warnings || exit $?
