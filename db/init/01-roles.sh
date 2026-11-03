#!/bin/bash
# Runs once when the Docker volume is first created. Creates the two application roles and the
# database. Passwords come from the environment (see .env.example); nothing is hardcoded.
set -euo pipefail
: "${TREASURY_OWNER_PASSWORD:?TREASURY_OWNER_PASSWORD must be set}"
: "${TREASURY_APP_PASSWORD:?TREASURY_APP_PASSWORD must be set}"

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname postgres \
     -v owner_pw="$TREASURY_OWNER_PASSWORD" -v app_pw="$TREASURY_APP_PASSWORD" <<-'EOSQL'
    -- Owns the schema, runs migrations and the test suite. CREATEDB is for per-test databases.
    CREATE ROLE treasury_owner LOGIN PASSWORD :'owner_pw' NOSUPERUSER CREATEDB NOCREATEROLE NOBYPASSRLS;
    -- What the running server connects as.
    CREATE ROLE treasury_app LOGIN PASSWORD :'app_pw' NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
    CREATE DATABASE treasury OWNER treasury_owner;
    REVOKE ALL ON DATABASE treasury FROM PUBLIC;
    GRANT CONNECT ON DATABASE treasury TO treasury_app;
EOSQL
