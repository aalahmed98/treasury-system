#!/usr/bin/env bash
# HTTP smoke test: login as each seeded user and show what each one can see and do.
# BASE may be https://...; set CACERT to a PEM file to pin a development certificate.
set -u
BASE=${BASE:-http://localhost:8787}
CURL="curl -s ${CACERT:+--cacert $CACERT}"
for i in $(seq 1 60); do $CURL -f "$BASE/healthz" >/dev/null && break; sleep 1; done
echo "healthz: $($CURL "$BASE/healthz")"
login() { $CURL -X POST "$BASE/auth/login" -H 'content-type: application/json' -d "{\"username\":\"$1\",\"password\":\"$2\"}"; }
tok() { login "$1" password | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])'; }
echo "--- bad password -> $(login fx1 wrong)"
for u in fx1 mm1 alm1 head1; do
  TOK=$(tok $u)
  echo "--- $u desks: $($CURL "$BASE/me" -H "authorization: Bearer $TOK" | python3 -c 'import sys,json; print([ (d["code"], d["can_book"]) for d in json.load(sys.stdin)["desks"]])')"
  $CURL "$BASE/deals" -H "authorization: Bearer $TOK" | python3 -c '
import sys,json,collections
r=json.load(sys.stdin); c=collections.Counter(d["desk_code"] for d in r["deals"]); print("    deals by desk:", dict(c), "total", len(r["deals"]))'
  $CURL "$BASE/positions" -H "authorization: Bearer $TOK" | python3 -c '
import sys,json; p=json.load(sys.stdin); print("    positions rows:", len(p), [ (x["desk_code"],x["ccy"],x["net_notional"]) for x in p[:3] ])'
done
echo "--- no token -> $($CURL -o /dev/null -w "%{http_code}" "$BASE/deals")"
MM=$(tok mm1); ALM=$(tok alm1)
FX_DEAL='{"desk_id":1,"product":"FX_SPOT","side":"BUY","ccy1":"USD","ccy2":"BHD","notional":"1000000","rate":"0.376","trade_date":"2026-11-03","value_date":"2026-11-05","maturity_date":null,"counterparty":"Test Bank"}'
MM_DEAL='{"desk_id":2,"product":"MM_DEPO","side":"LEND","ccy1":"BHD","ccy2":null,"notional":"1000000","rate":"5.1","trade_date":"2026-11-03","value_date":"2026-11-05","maturity_date":"2026-12-05","counterparty":"Test Bank"}'
post() { $CURL -w " [%{http_code}]" -X POST "$BASE/deals" -H "authorization: Bearer $1" -H 'content-type: application/json' -d "$2" | cut -c1-140; echo; }
echo "--- mm1 books into FX (expect 403):  $(post "$MM" "$FX_DEAL")"
echo "--- alm1 books into MM, read-only entitlement (expect 403 from RLS): $(post "$ALM" "$MM_DEAL")"
echo "--- mm1 books into MM (expect 201): $(post "$MM" "$MM_DEAL")"
