#!/usr/bin/env bash
# Stop the reconciler and the API started by start-dev.sh. The database container keeps running
# (docker compose stop to pause it, docker compose down -v to wipe it).
pkill -f "[t]reasury-recon serve" 2>/dev/null && echo "reconciler stopped" || echo "reconciler was not running"
pkill -f "[t]reasury-server serve" 2>/dev/null && echo "api server stopped" || echo "api server was not running"
