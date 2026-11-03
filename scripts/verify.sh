#!/usr/bin/env bash
# End-to-end check from a clean database: migrate, seed users, replay a synthetic world through
# the reconciler, start the API, then prove the reconciliation behaviour over HTTP:
#  - every break kind appears; deals carry match status and sources
#  - an internal booking followed by a Kondor observation with its cross reference links within 2 s
#  - a user resolution is refused for the wrong desk and applied by the worker for the right one
#  - a rebuild reproduces the same deal and link counts
#  - the live event stream delivers deal and break events to entitled desks only
set -euo pipefail
cd "$(dirname "$0")/.."
set -a; source .env; set +a
BASE=${BASE:-http://localhost:8787}
DAYS=${DAYS:-5}

docker compose down -v >/dev/null 2>&1 || true
docker compose up -d --wait
cargo build -q --workspace
cargo run -q -p server -- migrate
cargo run -q -p server -- seed
echo "--- synthetic world: $DAYS business days ---"
cargo run -q -p recon -- synth --mode replay --speed 20000 --days "$DAYS" 2>&1 | grep -E "world generated|deals by|^  |breaks:|event outcomes" | sed -E 's/\x1b\[[0-9;]*m//g'

cargo run -q -p recon -- serve >/tmp/treasury-recon.log 2>&1 &
RECON_PID=$!
cargo run -q -p server -- serve >/tmp/treasury-server.log 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID $RECON_PID 2>/dev/null || true' EXIT
for i in $(seq 1 60); do curl -sf "$BASE/healthz" >/dev/null && break; sleep 1; done

./scripts/smoke.sh

tok() { curl -s -X POST "$BASE/auth/login" -H 'content-type: application/json' \
          -d "{\"username\":\"$1\",\"password\":\"password\"}" | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])'; }
FX=$(tok fx1); MM=$(tok mm1); HEAD=$(tok head1); ALM=$(tok alm1)

echo "--- every break kind is present (head1) ---"
curl -s "$BASE/breaks?status=all" -H "authorization: Bearer $HEAD" | python3 -c '
import sys,json,collections; r=json.load(sys.stdin); c=collections.Counter(b["kind"] for b in r)
missing=[k for k in ("MISSING","MISMATCH","ORPHAN","AMBIGUOUS","DUPLICATE") if c.get(k,0)==0]
print("  breaks by kind:", dict(c)); print("  all kinds present:", "yes" if not missing else "NO, missing "+",".join(missing))'

echo "--- an internal booking becomes the anchor for a Kondor observation (cross reference) ---"
DEAL=$(curl -s -X POST "$BASE/deals" -H "authorization: Bearer $FX" -H 'content-type: application/json' \
  -d '{"desk_id":1,"product":"FX_SPOT","side":"BUY","ccy1":"USD","ccy2":"BHD","notional":"7000000","rate":"0.37600","trade_date":"'$(date -u +%F)'","value_date":"'$(date -u -d "+2 days" +%F)'","maturity_date":null,"counterparty":"Gulf International Bank"}' \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')
psql "$RECON_DATABASE_URL" -qAtc "INSERT INTO source_events (source, source_ref, event_kind, source_seq, source_ts, observed_at, payload, desk_id, dedup_hash)
  SELECT 'KONDOR', 'K-VERIFY', 'NEW', 1, now(), now(), \$\$ {\"deal_id\":\"K-VERIFY\",\"portfolio\":\"FX-BOOK\",\"instrument\":\"FXSPOT\",\"direction\":\"BUY\",\"ccy1\":\"USD\",\"ccy2\":\"BHD\",\"amount\":7000000,\"rate\":0.37600,\"trade_date\":\"$(date -u +%F)\",\"value_date\":\"$(date -u -d '+2 days' +%F)\",\"maturity_date\":null,\"counterparty\":\"GIB\",\"trader\":\"FXT1\",\"status\":\"NEW\",\"refs\":{}} \$\$::jsonb, id, sha256('verify-kondor'::bytea)
  FROM desks WHERE code = 'UNASSIGNED'"
sleep 2
curl -s "$BASE/deals/$DEAL/provenance" -H "authorization: Bearer $FX" | python3 -c '
import sys,json; p=json.load(sys.stdin); srcs=[s["source"] for s in p["sources"]]
print("  sources on the booking:", srcs, "| match:", p["deal"]["match_status"], "| linked within 2s:", "yes" if "KONDOR" in srcs else "NO")'

echo "--- resolving: alm1 (read-only) 403, mm1 (other desk) 404, fx1 202 then applied by the worker ---"
BID=$(curl -s "$BASE/breaks" -H "authorization: Bearer $FX" | python3 -c 'import sys,json; r=json.load(sys.stdin); print([b for b in r if b["kind"]=="ORPHAN"][0]["id"])')
echo "  alm1 -> $(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/breaks/$BID/ack" -H "authorization: Bearer $ALM")"
echo "  mm1  -> $(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/breaks/$BID/ack" -H "authorization: Bearer $MM")"
echo "  fx1  -> $(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/breaks/$BID/resolve" -H "authorization: Bearer $FX" -H 'content-type: application/json' -d '{"action":"DISMISS","note":"verify.sh"}')"
sleep 2
curl -s "$BASE/breaks/$BID" -H "authorization: Bearer $FX" | python3 -c 'import sys,json; d=json.load(sys.stdin)["break_"]; print("  break now:", d["status"], d["resolution"])'

echo "--- live events: fx1 and mm1 subscribe; one FX deal is inserted directly ---"
curl -s -N --max-time 6 "$BASE/events" -H "authorization: Bearer $FX" >/tmp/ev-fx1.txt & C1=$!
curl -s -N --max-time 6 "$BASE/events" -H "authorization: Bearer $MM" >/tmp/ev-mm1.txt & C2=$!
sleep 1.5
psql "$DATABASE_URL" -qAtc "INSERT INTO deals (id, deal_ref, desk_id, product, side, ccy1, ccy2, notional, rate, trade_date, value_date, counterparty, trader_id, status)
  SELECT gen_random_uuid(), 'VERIFY-' || floor(random()*1e9)::text, k.id, 'FX_SPOT', 'BUY', 'USD', 'BHD', 1000000, 0.376, current_date, current_date + 2, 'Verify Bank', u.id, 'BOOKED'
  FROM desks k, users u WHERE k.code = 'FX' AND u.username = 'fx1'"
wait $C1 $C2 || true
echo "  fx1 stream: $(grep -c 'event: deal' /tmp/ev-fx1.txt || true) deal event(s) (expect >= 1)   mm1 stream: $(grep -c 'event: deal' /tmp/ev-mm1.txt || true) (expect 0)"

echo "--- rebuild reproduces the projection ---"
BEFORE=$(psql "$DATABASE_URL" -Atc "select (select count(*) from deals where merged_into is null) || '/' || (select count(*) from source_deals where deal_id is not null)")
kill $RECON_PID; wait $RECON_PID 2>/dev/null || true
cargo run -q -p recon -- rebuild 2>&1 | grep -E "rebuild complete|user actions" | sed -E 's/\x1b\[[0-9;]*m//g'
AFTER=$(psql "$DATABASE_URL" -Atc "select (select count(*) from deals where merged_into is null) || '/' || (select count(*) from source_deals where deal_id is not null)")
echo "  deals/links before $BEFORE after $AFTER -> $([ "$BEFORE" = "$AFTER" ] && echo identical || echo DIFFERENT)"
