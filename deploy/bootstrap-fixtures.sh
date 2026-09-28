#!/bin/sh
# Source from the repository root after configuring five distinct database pairs.
# No credentials are written to evidence. The four upgrade databases MUST be empty.
set -eu
: "${FIXTURE_EVIDENCE_DIR:?set a fresh absolute evidence directory}"
mkdir -p "$FIXTURE_EVIDENCE_DIR"
export TEST_P0A_FIXTURE_MANIFEST="$FIXTURE_EVIDENCE_DIR/p0a.json"
export TEST_B1_FIXTURE_MANIFEST="$FIXTURE_EVIDENCE_DIR/b1.json"
export TEST_B3_SCHEMA_FIXTURE_MANIFEST="$FIXTURE_EVIDENCE_DIR/b3-schema.json"
export TEST_B2_FIXTURE_MANIFEST="$FIXTURE_EVIDENCE_DIR/b2.json"
for kind in p0a b1 b3-schema b2; do
    export FIXTURE_MANIFEST_OUTPUT="$FIXTURE_EVIDENCE_DIR/$kind.json"
    test ! -e "$FIXTURE_MANIFEST_OUTPUT"
    set +e
    cargo run --offline --locked --manifest-path deploy/fixture-runner/Cargo.toml -- "$kind" > "$FIXTURE_EVIDENCE_DIR/bootstrap-$kind.stdout.log" 2> "$FIXTURE_EVIDENCE_DIR/bootstrap-$kind.stderr.log"
    status=$?
    set -e
    printf '%s\n' "$status" > "$FIXTURE_EVIDENCE_DIR/bootstrap-$kind.exit"
    cat "$FIXTURE_EVIDENCE_DIR/bootstrap-$kind.stdout.log"
    cat "$FIXTURE_EVIDENCE_DIR/bootstrap-$kind.stderr.log" >&2
    if [ "$status" -ne 0 ]; then return "$status" 2>/dev/null || exit "$status"; fi
    chmod 444 "$FIXTURE_MANIFEST_OUTPUT"
done
sha256sum "$FIXTURE_EVIDENCE_DIR"/*.json > "$FIXTURE_EVIDENCE_DIR/fixtures.sha256"
unset FIXTURE_MANIFEST_OUTPUT
