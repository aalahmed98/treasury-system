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
FX=$(tok fx1); MM=$(tok mm1); HEAD=$(tok head1); ALM=$(tok alm1); PMU=$(tok pmu1)

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

echo "--- liquidity ladder: desk-scoped expected flows, nostro actuals for ALM/HEAD only ---"
HEAD_TOKEN=$HEAD FX_TOKEN=$FX BASE=$BASE python3 -I - <<'PYLADDER'
import json, os, urllib.request
base, H, F = os.environ["BASE"], os.environ["HEAD_TOKEN"], os.environ["FX_TOKEN"]
def get(path, tok):
    return json.load(urllib.request.urlopen(urllib.request.Request(base + path, headers={"authorization": "Bearer " + tok})))
h = get("/ladder?ccy=USD&from=2026-10-05&days=10", H)
f = get("/ladder?ccy=USD&from=2026-10-05&days=10", F)
with_actuals = sum(1 for r in h["rows"] if r["actual_net"] is not None)
print("  head1: access", h["liquidity_access"], "| opening", h["opening"]["balance"] if h["opening"] else None, "| rows with actuals", with_actuals, "| rows with deals", sum(1 for r in h["rows"] if r["deals"]))
print("  fx1:   access", f["liquidity_access"], "| opening", f["opening"], "| rows with actuals", sum(1 for r in f["rows"] if r["actual_net"] is not None), "| rows with deals", sum(1 for r in f["rows"] if r["deals"]))
print("  checks:", "ok" if h["liquidity_access"] and not f["liquidity_access"] and with_actuals > 0 and h["opening"] else "FAILED")
PYLADDER

echo "--- amend and history: fx1 amends its booking; mm1 cannot see it ---"
curl -s -o /dev/null -w "  fx1 amend -> %{http_code}\n" -X PATCH "$BASE/deals/$DEAL" -H "authorization: Bearer $FX" -H 'content-type: application/json' -d '{"counterparty":"Gulf International Bank B.S.C."}'
curl -s -o /dev/null -w "  mm1 amend -> %{http_code} (expect 404)\n" -X PATCH "$BASE/deals/$DEAL" -H "authorization: Bearer $MM" -H 'content-type: application/json' -d '{"counterparty":"X"}'
curl -s "$BASE/deals/$DEAL/history" -H "authorization: Bearer $FX" | python3 -c 'import sys,json; h=json.load(sys.stdin); print("  history versions:", [e["version"] for e in h], "| last change:", [(c["field"], c["to"]) for c in h[-1]["changes"]])'

echo "--- ALM workbench: synthetic banking book, parameters, calculators, scoping ---"
ALM=$(tok alm1)
echo "  snapshots $(psql "$DATABASE_URL" -Atc "select count(*) from bs_snapshots where complete") · cohorts $(psql "$DATABASE_URL" -Atc "select count(*) from bs_positions") · curve points $(psql "$DATABASE_URL" -Atc "select count(*) from market_rates") · income rows $(psql "$DATABASE_URL" -Atc "select count(*) from alm_period")"
ALM_TOKEN=$ALM FX_TOKEN=$FX BASE=$BASE python3 -I - <<'PYALM'
import json, os, urllib.request, urllib.error
base, A, F = os.environ["BASE"], os.environ["ALM_TOKEN"], os.environ["FX_TOKEN"]
def call(method, path, tok, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(base + path, data=data, method=method, headers={"authorization": "Bearer " + tok, "content-type": "application/json"})
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, None
ok = True
st, params = call("GET", "/alm/params", A); ok &= st == 200 and params["active"]["version"] >= 1
print("  alm1 parameters: HTTP", st, "| active version", params and params["active"]["version"])
st, _ = call("GET", "/alm/params", F); ok &= st == 404
print("  fx1 parameters: HTTP", st, "(expect 404)")
st, lcr = call("POST", "/alm/lcr", A, {}); ratio = lcr and lcr["total"]["ratio"]
ok &= st == 200 and ratio is not None and 50 <= ratio <= 400
print("  alm1 LCR: HTTP", st, "| ratio %.1f%%" % ratio if ratio else "", "| by currency", [c["ccy"] for c in lcr["by_ccy"]] if lcr else None)
st, _ = call("POST", "/alm/lcr", F, {}); ok &= st == 404
print("  fx1 LCR: HTTP", st, "(expect 404)")
for tool in ["nsfr", "gap", "nii", "stress", "funding", "attribution"]:
    st, r = call("POST", "/alm/" + tool, A, {}); ok &= st == 200
    print("  alm1 %s: HTTP %s" % (tool, st))
st, o = call("GET", "/alm/overview", A); ok &= st == 200 and len(o["limits"]) >= 8 and len(o["lcr_trend"]) >= 1
print("  alm1 overview: HTTP", st, "| limits", o and len(o["limits"]), "| trend points", o and len(o["lcr_trend"]), "| NII 12m %.1fm" % (o["nii_12m"] / 1e6) if o else "")
st, _ = call("GET", "/alm/overview", F); ok &= st == 404
print("  fx1 overview: HTTP", st, "(expect 404)")
p = params["params"]; p["lcr"]["internal_minimum"] = 500.0
st, _ = call("POST", "/alm/params", F, {"params": p, "note": "verify"}); ok &= st == 404
print("  fx1 save parameters: HTTP", st, "(expect 404)")
st, v = call("POST", "/alm/params", A, {"params": p, "note": "verify: internal minimum raised to 500%"}); ok &= st == 201
print("  alm1 save parameters: HTTP", st, "| new version", v and v["version"])
st, items = call("GET", "/attention", A); alm_items = [i for i in (items or []) if i["target"].get("kind") == "alm"]
ok &= st == 200 and any(i["title"].startswith("LCR") and i["severity"] == "CRITICAL" for i in alm_items)
print("  alm1 attention: HTTP", st, "|", len(items or []), "items |", [i["title"] for i in alm_items][:3])
st, fitems = call("GET", "/attention", F); ok &= st == 200 and not any(i["target"].get("kind") == "alm" for i in (fitems or []))
print("  fx1 attention: HTTP", st, "|", len(fitems or []), "items, none about ALM")
st, acts = call("GET", "/activity?limit=10", A); ok &= st == 200 and any(a["kind"] == "parameters" for a in (acts or []))
print("  alm1 activity: HTTP", st, "|", [a["kind"] for a in (acts or [])][:5])
st, m = call("GET", "/market/rates", F); ok &= st == 200 and len(m["curves"]) >= 1
print("  fx1 market rates: HTTP", st, "|", len(m["curves"]) if m else 0, "curves,", len(m["spots"]) if m else 0, "spots")
print("  checks:", "ok" if ok else "FAILED")
PYALM

echo "--- FX risk: positions, P&L, VaR and limits from the visible deals; limits set by the head only ---"
python3 - "$BASE" "$FX" "$MM" "$HEAD" <<'PY'
import json, sys, urllib.request, urllib.error
base, fx, mm, head = sys.argv[1:5]
def call(path, tok, body=None):
    data = None if body is None else json.dumps(body).encode()
    r = urllib.request.Request(base + path, data=data, headers={"authorization": "Bearer " + tok, "content-type": "application/json"}, method="POST" if body is not None else "GET")
    try:
        with urllib.request.urlopen(r) as res: return res.status, json.loads(res.read() or b"null")
    except urllib.error.HTTPError as e: return e.code, None
s, r = call("/risk/fx", fx)
print(f"  fx1 risk: HTTP {s} | positions {len(r['positions'])} | NOP {r['nop_total']/1e6:.1f}m | VaR {r['var']['var99']/1e3:.0f}k | limits {len(r['limits'])}")
s2, m = call("/risk/fx", mm)
print(f"  mm1 risk: HTTP {s2} | positions {len(m['positions'])} (expect 0)")
_, l = call("/risk/limits", fx)
s3, _ = call("/risk/limits", fx, {"limits": l["limits"], "note": "verify: dealer may not"})
print(f"  fx1 save limits: HTTP {s3} (expect 403) | head1 may edit: {call('/risk/limits', head)[1]['can_edit']}")
s4, h = call("/risk/fx/history?days=10", fx)
print(f"  history: HTTP {s4} | {len(h['points'])} days")
ok = s == 200 and len(r["positions"]) > 0 and s2 == 200 and len(m["positions"]) == 0 and s3 == 403 and s4 == 200
print("  checks:", "ok" if ok else "FAILED")
PY

echo "--- ALM additions: liquidity ladder, early warnings, the ALCO pack ---"
python3 - "$BASE" "$ALM" "$FX" <<'PY'
import json, sys, urllib.request, urllib.error
base, alm, fx = sys.argv[1:4]
def call(path, tok, body=None):
    data = None if body is None else json.dumps(body).encode()
    r = urllib.request.Request(base + path, data=data, headers={"authorization": "Bearer " + tok, "content-type": "application/json"}, method="POST" if body is not None else "GET")
    try:
        with urllib.request.urlopen(r) as res: return res.status, json.loads(res.read() or b"null")
    except urllib.error.HTTPError as e: return e.code, None
s1, g = call("/alm/liquidity-gap", alm, {})
s2, e = call("/alm/ewi", alm, {})
s3, p = call("/alm/pack", alm)
f1, _ = call("/alm/liquidity-gap", fx, {})
f2, _ = call("/alm/pack", fx)
print(f"  alm1 ladder: HTTP {s1} | buckets {len(g['behavioural']['buckets'])} | early warnings: HTTP {s2} stage {e['stage']} | pack: HTTP {s3} {p['period']}")
print(f"  fx1 ladder: HTTP {f1} | pack: HTTP {f2} (expect 404)")
print("  checks:", "ok" if s1 == s2 == s3 == 200 and f1 == f2 == 404 else "FAILED")
PY

echo "--- FTP curves: ALM publishes from its workbook, every desk reads them ---"
python3 - "$BASE" "$ALM" "$FX" <<'PY'
import json, sys, urllib.request, urllib.error
base, alm, fx = sys.argv[1:4]
def call(path, tok, body=None, raw=None):
    data = raw if raw is not None else (None if body is None else json.dumps(body).encode())
    ct = "application/octet-stream" if raw is not None else "application/json"
    r = urllib.request.Request(base + path, data=data, headers={"authorization": "Bearer " + tok, "content-type": ct}, method="POST" if data is not None else "GET")
    try:
        with urllib.request.urlopen(r) as res:
            b = res.read()
            return res.status, (json.loads(b) if res.headers.get_content_type() == "application/json" and b else b)
    except urllib.error.HTTPError as e: return e.code, None
s1, c = call("/ftp", fx)
cur = c["current"]
print(f"  fx1 reads: HTTP {s1} | in force {cur['as_of']} | {', '.join(x['ccy'] for x in cur['curves'])} | stale {c['stale']}")
s2, book = call("/ftp/template.xlsx", alm)
s3, prev = call("/ftp/preview", alm, raw=book)
print(f"  alm1 template: HTTP {s2} ({len(book)} bytes) | preview: HTTP {s3} | {len(prev['curves'])} curves, {len(prev['errors'])} errors, layout {prev['layout']!r}")
s4, pub = call("/ftp/publish", alm, {"as_of": prev["as_of"], "curves": [], "upload_id": prev["upload_id"], "note": "verify: the curves in force, republished"})
s5, _ = call("/ftp/preview", fx, raw=book)
print(f"  alm1 publish from the upload log: HTTP {s4} (expect 201) | fx1 preview: HTTP {s5} (expect 403)")
s6, log = call("/ftp/log?limit=5&files_only=true", fx)
top = log["entries"][0] if s6 == 200 and log["entries"] else {}
s7, stored = call(f"/ftp/upload/{prev['upload_id']}/file", fx)
print(f"  fx1 upload log: HTTP {s6} | newest {top.get('status')} by {top.get('by')} | file download: HTTP {s7}, identical {stored == book}")
ok = (s1 == 200 and len(cur["curves"]) >= 4 and s2 == 200 and book[:2] == b"PK" and s3 == 200 and not prev["errors"] and s4 == 201 and s5 == 403
      and top.get("status") == "in_force" and top.get("upload_id") == prev["upload_id"] and s7 == 200 and stored == book)
print("  checks:", "ok" if ok else "FAILED")
PY

echo "--- PMU desk: the bond portfolio, its income, risk and funding ---"
python3 - "$BASE" "$PMU" "$FX" <<'PY'
import json, sys, time, urllib.request, urllib.error
base, pmu, fx = sys.argv[1:4]
def call(path, tok):
    r = urllib.request.Request(base + path, headers={"authorization": "Bearer " + tok})
    t = time.time()
    try:
        with urllib.request.urlopen(r) as res: return res.status, json.loads(res.read()), time.time() - t
    except urllib.error.HTTPError as e: return e.code, None, time.time() - t
ok = True
slow = []
res = {}
for p in ["overview", "positions", "income", "forecast?reinvest=true", "risk", "funding"]:
    s, body, dt = call("/pmu/" + p, pmu)
    res[p.split("?")[0]] = body
    ok &= s == 200
    if dt > 1.0: slow.append(f"{p} {dt:.2f}s")
o = res["overview"]["headline"]
print(f"  pmu1 overview: face {o['portfolio_face_bhd']/1e6:.0f}m | lots {o['lots']} | MTD carry {o['mtd_carry_bhd']/1e3:.1f}k | net DV01 {o['net_dv01_bhd']/1e3:.1f}k | yield {o['portfolio_yield']:.2f}%")
levels = {r["level"] for r in res["positions"]["rows"]}
t = res["income"]["total"]
adds = abs(t["carry"] - (t["coupon"] + t["amortisation"] - t["ftp"])) < 1e-3 and abs(t["net_nii"] - (t["carry"] + t["irs"])) < 1e-3
r = res["risk"]
print(f"  positions: {len(res['positions']['rows'])} rows, levels {sorted(levels)} | income parts add up: {adds} | hedge pairs {len(r['hedge_pairs'])} | shocks {len(r['shocks'])}")
f = res["funding"]
print(f"  funding: {f['summary']['active']} repos, {f['summary']['amount_bhd']/1e6:.0f}m | quotes {len(f['quotes'])} | forecast full year {res['forecast']['full_year']['net_nii']/1e6:.2f}m")
s, _, _ = call("/pmu/overview", fx)
print(f"  fx1 overview: HTTP {s} (expect 404){' | slow: ' + ', '.join(slow) if slow else ''}")
ok &= o["lots"] > 0 and levels == {"BOOK", "BOND", "LOT"} and adds and len(r["shocks"]) == 6 and f["summary"]["active"] > 0 and s == 404 and not slow
print("  checks:", "ok" if ok else "FAILED")
PY

echo "--- PMU booking: buy by ISIN, sell oldest first, repo against free face; only the desk's bookers ---"
python3 - "$BASE" "$PMU" "$ALM" <<'PY'
import json, sys, urllib.request, urllib.error
base, pmu, alm = sys.argv[1:4]
def call(path, tok, body=None, method=None):
    data = None if body is None else json.dumps(body).encode()
    r = urllib.request.Request(base + path, data=data, headers={"authorization": "Bearer " + tok, "content-type": "application/json"}, method=method or ("POST" if data else "GET"))
    try:
        with urllib.request.urlopen(r) as res: return res.status, json.loads(res.read() or b"null")
    except urllib.error.HTTPError as e:
        try: return e.code, json.loads(e.read() or b"null")
        except Exception: return e.code, None
_, hits = call("/pmu/bonds?q=saudi%20sov", pmu)
held = next(h for h in hits if h["holdings"])
isin = held["bond"]["isin"]; book = held["holdings"][0][0]
buy = {"side": "BUY", "isin": isin, "book": book, "face": 2000000, "clean_price": held["market_price"]}
s1, pv = call("/pmu/ticket/preview", pmu, buy)
s2, b = call("/pmu/buy", pmu, buy)
print(f"  pmu1 preview {isin}: HTTP {s1} | yield {pv['quote']['ytm']:.3f}% | EIR {pv['quote']['eir']:.3f}% | FTP {pv['quote']['ftp_rate']} | carry {pv['quote']['carry_bp']:.0f} bp | buy: HTTP {s2}")
sell = {"side": "SELL", "isin": isin, "face": 1000000, "clean_price": held["market_price"]}
s3, sp = call("/pmu/ticket/preview", pmu, sell)
s4, _ = call("/pmu/sell", pmu, sell)
print(f"  sell 1m: preview HTTP {s3} | {len(sp['legs'])} lot(s), realised {sp['realised_bhd']:.0f} BHD | book: HTTP {s4}")
import datetime
start = datetime.date.today().isoformat(); end = (datetime.date.today() + datetime.timedelta(days=30)).isoformat()
rp = {"direction": "REPO", "counterparty": "Citibank N.A.", "ccy": "USD", "start": start, "end": end, "rate": 4.6, "haircut_pct": 5, "collateral": []}
_, rpv = call("/pmu/repo/preview", pmu, rp)
cand = next(c for c in rpv["available"] if c["ccy"] == "USD")
rp["collateral"] = [{"lot_id": cand["lot_id"], "face": min(cand["free"], 5000000)}]
s5, r = call("/pmu/repo", pmu, rp)
s6, _ = call("/pmu/buy", alm, buy)
s7, _ = call(f"/pmu/lot/{b['id']}", pmu, {"kondor_id": "KND-VERIFY"}, "PATCH")
print(f"  repo on {cand['name']}: HTTP {s5} | alm1 buy: HTTP {s6} (expect 403) | Kondor id attached: HTTP {s7}")
ok = s1 == 200 and s2 == 201 and s3 == 200 and s4 == 201 and s5 == 201 and s6 == 403 and s7 == 200 and sp["legs"]
print("  checks:", "ok" if ok else "FAILED")
PY

echo "--- web client: the pages are served at /app, deep links included ---"
if [ -f web/dist/index.html ]; then
  APP=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/app/")
  DEEP=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/app/alm/lcr")
  ROOT=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/")
  echo "  GET /app/ -> $APP · deep link /app/alm/lcr -> $DEEP · GET / -> $ROOT (redirect)"
  echo "  checks: $([ "$APP" = 200 ] && [ "$DEEP" = 200 ] && [ "$ROOT" = 307 ] && curl -s "$BASE/app/alm/lcr" | grep -q 'id="root"' && echo ok || echo FAILED)"
else
  echo "  web/dist is missing: build it on Windows with scripts\\build-web.cmd (check skipped)"
fi

echo "--- rebuild reproduces the projection ---"
BEFORE=$(psql "$DATABASE_URL" -Atc "select (select count(*) from deals where merged_into is null) || '/' || (select count(*) from source_deals where deal_id is not null)")
kill $RECON_PID; wait $RECON_PID 2>/dev/null || true
cargo run -q -p recon -- rebuild 2>&1 | grep -E "rebuild complete|user actions" | sed -E 's/\x1b\[[0-9;]*m//g'
AFTER=$(psql "$DATABASE_URL" -Atc "select (select count(*) from deals where merged_into is null) || '/' || (select count(*) from source_deals where deal_id is not null)")
echo "  deals/links before $BEFORE after $AFTER -> $([ "$BEFORE" = "$AFTER" ] && echo identical || echo DIFFERENT)"
