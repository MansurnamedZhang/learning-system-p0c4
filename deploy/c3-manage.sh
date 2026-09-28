#!/bin/sh
set -eu
# Management only. Worker never receives this script, image or admin secret.
export TEST_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_c3_source_test"
export TEST_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_c3_source_test"
export C3_TARGET_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_c3_target_test"
export C3_TARGET_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_c3_target_test"
export ASSET_ROOT=/assets STAGING_ROOT=/staging SNAPSHOT_ROOT=/snapshots C3_CONTROL=/control C3_TARGET_ROOT=/destination
exec /app/target/debug/examples/c3_acceptance "$@"
