#!/bin/sh
set -eu
# Generated secret files contain hexadecimal passwords, so URL encoding is unambiguous.
export TEST_ADMIN_DATABASE_URL="postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/learning_test"
export TEST_DATABASE_URL="postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/learning_test"
exec cargo test --offline --locked --workspace -- --test-threads=1
