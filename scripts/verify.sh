#!/usr/bin/env bash
# End-to-end check from a clean database: migrate, seed, start server, smoke-test the API,
# then prove a deal inserted directly in Postgres reaches an entitled event-stream subscriber only.
set -euo pipefail
cd "$(dirname "$0")/.."
set -a; source .env; set +a
BASE=${BASE:-http://localhost:8787}

docker compose up -d --wait
cargo run -q -p server -- migrate
cargo run -q -p server -- seed

cargo run -q -p server -- serve >/tmp/treasury-server.log 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

./scripts/smoke.sh

tok() { curl -s -X POST "$BASE/auth/login" -H 'content-type: application/json' \
          -d "{\"username\":\"$1\",\"password\":\"password\"}" | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])'; }
FX=$(tok fx1); MM=$(tok mm1)

echo "--- live events: fx1 and mm1 subscribe, then one FX deal is inserted via psql ---"
curl -s -N --max-time 6 "$BASE/events" -H "authorization: Bearer $FX" >/tmp/ev-fx1.txt &
C1=$!
curl -s -N --max-time 6 "$BASE/events" -H "authorization: Bearer $MM" >/tmp/ev-mm1.txt &
C2=$!
sleep 1.5
psql "$DATABASE_URL" -qAtc "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status)
  SELECT gen_random_uuid(), 'VERIFY-' || floor(random()*1e9)::text, k.id, 'FX_SPOT', 'BUY', 'USD', 'BHD', 1000000, 0.376, current_date, current_date + 2, 'Verify Bank', u.id, 'BOOKED'
  FROM desks k, users u WHERE k.code = 'FX' AND u.username = 'fx1'"
wait $C1 $C2 || true
echo "fx1 stream: $(grep -c 'event: deal' /tmp/ev-fx1.txt || true) deal event(s)   (expect 1)"
echo "mm1 stream: $(grep -c 'event: deal' /tmp/ev-mm1.txt || true) deal event(s)   (expect 0)"
