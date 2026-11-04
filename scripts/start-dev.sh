#!/usr/bin/env bash
# Start the whole stack for development from WSL: Postgres, migrations, the reconciler and the API.
# Run it from a WSL terminal, then open http://localhost:8787/app/ or the desktop shell on Windows.
# Logs: /tmp/treasury-recon.log and /tmp/treasury-server.log. Stop with: ./scripts/stop-dev.sh
set -euo pipefail
cd "$(dirname "$0")/.."
if [ ! -f .env ]; then cp .env.example .env; echo "created .env from .env.example"; fi
set -a; source .env; set +a

echo "1/5 database"; docker compose up -d --wait >/dev/null
echo "2/5 build";    cargo build -q --workspace
echo "3/5 migrate";  cargo run -q -p server -- migrate 2>&1 | grep -oE "migrations applied|error.*" || true
echo "4/5 seed";     cargo run -q -p server -- seed 2>&1 | grep -oE "users seeded|error.*" || true
DEALS=$(psql "$DATABASE_URL" -Atc "select count(*) from deals" 2>/dev/null || echo 0)
if [ "$DEALS" = "0" ]; then
  echo "    no deals yet: replaying a 5-day synthetic world (about 40 s)"
  cargo run -q -p recon -- synth --mode replay --speed 20000 --days 5 2>&1 | grep -E "deals by|^  " | sed -E 's/\x1b\[[0-9;]*m//g' || true
fi
# The balance sheet and the market up to the business date; snapshots already there are kept, so
# this only adds the days since the last run (a few seconds).
echo "    banking book and market data up to the business date"
cargo run -q -p recon -- alm-synth 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g' | grep -o "snapshots=.*" | tail -1 || true
echo "5/5 services"
pkill -f "[t]reasury-recon serve" 2>/dev/null || true
pkill -f "[t]reasury-server serve" 2>/dev/null || true
nohup cargo run -q -p recon -- serve >/tmp/treasury-recon.log 2>&1 &
nohup cargo run -q -p server -- serve >/tmp/treasury-server.log 2>&1 &
for i in $(seq 1 60); do curl -sf "http://localhost:${BIND_ADDR##*:}/healthz" >/dev/null && break; sleep 1; done
echo
echo "API:         http://${BIND_ADDR}   (healthz: $(curl -s "http://localhost:${BIND_ADDR##*:}/healthz" || echo down))"
echo "Reconciler:  running (log /tmp/treasury-recon.log)"
echo "Browser:     http://localhost:8787/app/   (after scripts\\build-web.cmd on Windows)"
echo "Desktop:     dist\\NBB-Treasury-Setup.exe (the web client in a window of its own)"
echo "Log in as:   fx1, fx2, mm1, alm1 or head1   password: password"
