#!/bin/sh
set -eu

# Only the provisioner supplies this validated UUID-derived name. psql quotes it
# as an SQL identifier because UUID hyphens are not bare identifier characters.
psql -v ON_ERROR_STOP=1 -v "dbname=$C4_TARGET_DATABASE" \
  --username "$POSTGRES_USER" --dbname postgres <<'SQL'
\set admin_password `cat /run/secrets/admin_password`
CREATE ROLE learning_admin LOGIN PASSWORD :'admin_password' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
CREATE ROLE learning_runtime NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
CREATE DATABASE :"dbname" OWNER learning_admin TEMPLATE template0;
REVOKE ALL ON DATABASE :"dbname" FROM PUBLIC;
GRANT CONNECT, TEMPORARY ON DATABASE :"dbname" TO learning_admin;
SQL

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" \
  --dbname "$C4_TARGET_DATABASE" <<'SQL'
REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO PUBLIC;
REVOKE ALL ON FUNCTION pg_catalog.pg_control_system() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO learning_admin;
SQL
