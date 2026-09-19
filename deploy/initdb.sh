#!/bin/sh
set -eu
psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname postgres <<'SQL'
\set admin_password `cat /run/secrets/admin_password`
\set runtime_password `cat /run/secrets/runtime_password`
CREATE ROLE learning_admin LOGIN PASSWORD :'admin_password' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
CREATE ROLE learning_runtime LOGIN PASSWORD :'runtime_password' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
CREATE ROLE learning_auth_lock NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
GRANT learning_auth_lock TO learning_admin WITH INHERIT FALSE, SET TRUE;
CREATE DATABASE learning_test OWNER learning_admin;
REVOKE ALL ON DATABASE learning_test FROM PUBLIC;
GRANT CONNECT, TEMPORARY ON DATABASE learning_test TO learning_admin, learning_runtime;
CREATE DATABASE learning_upgrade_test OWNER learning_admin;
REVOKE ALL ON DATABASE learning_upgrade_test FROM PUBLIC;
GRANT CONNECT, TEMPORARY ON DATABASE learning_upgrade_test TO learning_admin, learning_runtime;
SQL
