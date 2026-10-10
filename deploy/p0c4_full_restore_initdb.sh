#!/bin/sh
set -eu

# Dedicated full-restore cluster recipe. The separate root provisioner validates
# the package first and applies its safe INHERIT/connection limits before birth.
# Passwords stay in existing private secret mounts. Runtime has no CONNECT.
psql -v ON_ERROR_STOP=1 -v "dbname=$C4_TARGET_DATABASE" \
  --username "$POSTGRES_USER" --dbname postgres <<'SQL'
\set admin_password `cat /run/secrets/admin_password`
CREATE ROLE learning_admin LOGIN INHERIT CONNECTION LIMIT -1 PASSWORD :'admin_password' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOREPLICATION;
CREATE ROLE learning_auth_lock NOLOGIN INHERIT CONNECTION LIMIT -1 NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOREPLICATION;
CREATE ROLE learning_runtime LOGIN INHERIT CONNECTION LIMIT -1 NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOREPLICATION;
GRANT learning_auth_lock TO learning_admin WITH INHERIT FALSE, SET TRUE, ADMIN FALSE;
CREATE DATABASE :"dbname" OWNER learning_admin TEMPLATE template0;
REVOKE ALL ON DATABASE :"dbname" FROM PUBLIC;
REVOKE ALL ON DATABASE :"dbname" FROM learning_runtime;
GRANT CONNECT, TEMPORARY ON DATABASE :"dbname" TO learning_admin;
SQL

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" \
  --dbname "$C4_TARGET_DATABASE" <<'SQL'
REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO PUBLIC;
REVOKE ALL ON FUNCTION pg_catalog.pg_control_system() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO learning_admin;
SQL
