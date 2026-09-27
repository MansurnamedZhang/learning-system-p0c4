#!/bin/sh
# Source only inside the isolated management/test container. No set -x.
set -eu
while read -r prefix database; do
    export "${prefix}_ADMIN_DATABASE_URL=postgres://learning_admin:$(cat /run/secrets/admin_password)@pg/$database"
    export "${prefix}_DATABASE_URL=postgres://learning_runtime:$(cat /run/secrets/runtime_password)@pg/$database"
done <<'DATABASES'
TEST learning_test
TEST_UPGRADE learning_upgrade_test
TEST_B1_UPGRADE learning_b1_upgrade_test
TEST_B2_UPGRADE learning_b2_upgrade_test
TEST_B3_SCHEMA_UPGRADE learning_b3_schema_upgrade_test
TEST_C3_UPGRADE learning_c3_upgrade_test
TEST_IMPORT_A learning_import_a_test
TEST_IMPORT_B learning_import_b_test
TEST_IMPORT_UPGRADE learning_import_upgrade_test
C3_SOURCE learning_c3_source_test
C3_TARGET learning_c3_target_test
DATABASES
export TEST_SUPERUSER_DATABASE_URL="postgres://postgres:$(cat /run/secrets/postgres_password)@pg/learning_test"
