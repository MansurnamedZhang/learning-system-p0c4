#!/bin/sh
set -eu
# Generated secret files contain hexadecimal passwords, so URL encoding is unambiguous.
export TEST_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_test"
export TEST_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_test"
export TEST_UPGRADE_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_upgrade_test"
export TEST_UPGRADE_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_upgrade_test"
export TEST_B1_UPGRADE_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_b1_upgrade_test"
export TEST_B1_UPGRADE_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_b1_upgrade_test"
export TEST_B3_SCHEMA_UPGRADE_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_b3_schema_upgrade_test"
export TEST_B3_SCHEMA_UPGRADE_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_b3_schema_upgrade_test"
export TEST_B2_UPGRADE_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_b2_upgrade_test"
export TEST_B2_UPGRADE_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_b2_upgrade_test"
export FIXTURE_EVIDENCE_DIR=/evidence
. /app/deploy/bootstrap-fixtures.sh
set +e
cargo test --offline --locked --workspace -- --test-threads=1 > /evidence/workspace.stdout.log 2> /evidence/workspace.stderr.log
status=$?
set -e
printf '%s\n' "$status" > /evidence/workspace.exit
cat /evidence/workspace.stdout.log
cat /evidence/workspace.stderr.log >&2
sha256sum /evidence/*.json /evidence/*.log /evidence/*.exit > /evidence/results.sha256
exit "$status"
