#!/bin/bash
# offline tests: stubs stand in for curl and endlessh, nothing touches the network
# shellcheck disable=SC2016 # single quotes are deliberate: snippets run in a child bash
set -u
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir "$WORK/bin"
cp "$ROOT/tests/stub-curl" "$WORK/bin/curl"
cp "$ROOT/tests/stub-endlessh" "$WORK/bin/endlessh"
chmod +x "$WORK/bin/"*

pass=0 fail=0
ok() { # description, command...
  local desc=$1; shift
  if "$@" >/dev/null 2>&1; then pass=$((pass + 1)); else fail=$((fail + 1)); echo "FAIL: $desc"; fi
}
no() { ! "$@"; }

# --- unit: ip helpers
# shellcheck source=lib/ip.sh
. "$ROOT/lib/ip.sh"
ok "parses mapped ipv4" bash -c '. "$0/lib/ip.sh"; extract_ip "ACCEPT host=::ffff:1.2.3.4 port=9" && [ "$IP" = 1.2.3.4 ]' "$ROOT"
ok "parses + lowercases ipv6" bash -c '. "$0/lib/ip.sh"; extract_ip "ACCEPT host=2001:DB8::1 port=9" && [ "$IP" = 2001:db8::1 ]' "$ROOT"
for bad in 'host=1.2.3.4;touch${IFS}x' 'host=$(id)' 'host=999.1.1.1' 'host=1.2.3' 'host=`id`' 'host=1.2.3.4|id'; do
  ok "rejects $bad" no extract_ip "ACCEPT $bad port=9"
done
for r in 10.1.1.1 172.16.0.1 172.31.255.1 192.168.1.1 100.64.0.1 127.0.0.1 169.254.1.1 0.0.0.0 224.0.0.1 ::1 :: fe80::1 fd00::1 ff02::1; do
  ok "reserved: $r" is_reserved "$r"
done
for p in 8.8.8.8 172.32.0.1 100.128.0.1 223.1.1.1 2001:db8::1 2a00:1450::1; do
  ok "public: $p" no is_reserved "$p"
done

# --- e2e: run the real script against the stubs
run() { # name; env STUB_* set by caller; prints combined output
  : > "$WORK/log"; : > "$WORK/reported"
  PATH="$WORK/bin:$PATH" STUB_LOG="$WORK/log" STUB_CODES="$WORK/codes" STUB_LINES="$WORK/lines" \
    REPORTED_FILE="$WORK/reported" bash "$ROOT/tarpitReporter.sh" 2>&1
}
accept() { printf '2026-01-01T00:00:00Z ACCEPT host=%s port=1 fd=4 n=1/4096\n' "$1"; }
calls() { grep -c 'ARGV:.*/report' "$WORK/log"; }

{ accept ::ffff:1.1.1.1; accept ::ffff:1.1.1.1; accept 2.2.2.2; accept ::ffff:10.0.0.5
  accept '3.3.3.3;touch${IFS}pwned'; accept 4.4.4.4; echo "2026 CLOSE host=1.1.1.1 port=1 time=1 bytes=0"; } > "$WORK/lines"
: > "$WORK/codes"
out=$(API_TOKEN=secrettoken ALLOWLIST=4.4.4.4 run)
ok "reports each public ip once (dedupe, private, hostile, allowlist)" test "$(calls)" = 2
ok "audit file has 2 ips" test "$(grep -c . "$WORK/reported")" = 2
ok "dedupe logged" grep -q 'skipped (reported' <<< "$out"
ok "allowlist logged" grep -q '4.4.4.4 -> skipped (allowlisted)' <<< "$out"
ok "hostile line ignored" grep -q 'ignoring unparsable' <<< "$out"
ok "no injected file" no test -e pwned
ok "token sent via -K config" grep -q 'CFG: header = "Key: secrettoken"' "$WORK/log"
ok "token never in argv" no grep -q 'ARGV:.*secrettoken' "$WORK/log"

out=$(API_TOKEN=secrettoken DRY_RUN=1 run)
ok "dry-run sends nothing" test ! -s "$WORK/log"
ok "dry-run logs" grep -q 'dry-run, not sent' <<< "$out"

printf '200\n429\n' > "$WORK/codes"   # startup check ok, first report hits quota
out=$(API_TOKEN=secrettoken STUB_RETRY_AFTER=120 run)
ok "429 pauses reporting" grep -q 'pausing reports for 120s' <<< "$out"
ok "429: only one report attempted" test "$(calls)" = 1
ok "429: nothing recorded as reported" test ! -s "$WORK/reported"

printf '401\n' > "$WORK/codes"
out=$(API_TOKEN=badtoken run); rc=$?
ok "startup 401 is fatal" grep -q 'rejected the API token' <<< "$out"
ok "startup 401 exits 78" test "$rc" = 78

out=$(API_TOKEN='' run)
ok "missing token is fatal" grep -q 'API_TOKEN is not set' <<< "$out"
out=$(API_TOKEN=x ALLOWLIST=10.0.0.0/8 run)
ok "cidr allowlist rejected" grep -q 'plain ips' <<< "$out"

out=$(API_TOKEN_FILE="$WORK/does-not-exist" run); rc=$?
ok "unreadable token file dies with 78" test "$rc" = 78
ok "unreadable token file explains" grep -q 'cannot read API_TOKEN_FILE' <<< "$out"
echo -n 'filetoken' > "$WORK/tokfile"
: > "$WORK/codes"; out=$(API_TOKEN_FILE="$WORK/tokfile" run)
ok "token file is used" grep -q 'CFG: header = "Key: filetoken"' "$WORK/log"

printf '500\n' > "$WORK/codes"   # api down at startup: warn, keep running
out=$(API_TOKEN=secrettoken run)
ok "api error at startup is not fatal" grep -q 'continuing' <<< "$out"

# --- installer helpers
inst() { # code, snippet: run a snippet with install.sh sourced and curl stubbed
  echo "$1" > "$WORK/codes"; : > "$WORK/log"
  PATH="$WORK/bin:$PATH" STUB_LOG="$WORK/log" STUB_CODES="$WORK/codes" \
    bash -c ". \"\$0/install.sh\"; $2" "$ROOT" 2>/dev/null
}
ok "token_status returns http code" test "$(inst 200 'token_status tok123')" = 200
ok "installer token not in argv" no grep -q 'ARGV:.*tok123' "$WORK/log"
ok "installer token sent via stdin config" grep -q 'CFG: header = "Key: tok123"' "$WORK/log"
ok "token_status passes 401 through" test "$(inst 401 'token_status bad')" = 401
ok "--yes with rejected token dies" no inst 401 'YES=1 API_TOKEN=bad get_token'
ok "--yes with good token passes" inst 200 'YES=1 API_TOKEN=good get_token'
ok "unreachable api is only a warning" inst 000 'YES=1 API_TOKEN=x get_token'

echo "passed: $pass, failed: $fail"
[ "$fail" = 0 ]
