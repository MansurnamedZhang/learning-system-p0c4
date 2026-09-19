#!/bin/sh
set -eu
# Generated secret files contain hexadecimal passwords, so URL encoding is unambiguous.
export TEST_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_test"
export TEST_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_test"
export TEST_UPGRADE_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_upgrade_test"
export TEST_UPGRADE_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_upgrade_test"
export TEST_B1_UPGRADE_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_b1_upgrade_test"
export TEST_B1_UPGRADE_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_b1_upgrade_test"
TEST_P0A_MIGRATIONS_DIR=$(mktemp -d)
export TEST_P0A_MIGRATIONS_DIR
cp /app/migrations/0001_content_core.sql "$TEST_P0A_MIGRATIONS_DIR/"
TEST_B1_MIGRATIONS_DIR=$(mktemp -d)
export TEST_B1_MIGRATIONS_DIR
cp /app/migrations/0001_content_core.sql /app/migrations/0002_composition_release.sql "$TEST_B1_MIGRATIONS_DIR/"
exec cargo test --offline --locked --workspace -- --test-threads=1
