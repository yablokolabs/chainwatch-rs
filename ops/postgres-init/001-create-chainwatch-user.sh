#!/usr/bin/env sh
set -eu

APP_USER="${CHAINWATCH_DB_USER:-chainwatch_app}"
case "$APP_USER" in
  ""|*[!A-Za-z0-9_]*)
    echo "invalid app database user name" >&2
    exit 1
    ;;
esac

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<SQL
DO \$\$
BEGIN
   IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = '${APP_USER}') THEN
      CREATE ROLE ${APP_USER} LOGIN;
   END IF;
END
\$\$;
GRANT CONNECT ON DATABASE ${POSTGRES_DB} TO ${APP_USER};
GRANT USAGE, CREATE ON SCHEMA public TO ${APP_USER};
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO ${APP_USER};
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO ${APP_USER};
SQL
