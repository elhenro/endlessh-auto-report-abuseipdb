#!/bin/bash
# ssh tarpit (endlessh) that reports every visiting ip to AbuseIPDB
set -eu
cd "$(dirname "${BASH_SOURCE[0]}")"

if (( BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 4) )); then
  echo "tarpit: error: bash 4.4+ required" >&2
  exit 78
fi

# shellcheck source=lib/common.sh
. ./lib/common.sh
# shellcheck source=lib/ip.sh
. ./lib/ip.sh
# shellcheck source=lib/abuseipdb.sh
. ./lib/abuseipdb.sh

: "${TARPIT_PORT:=2222}" "${TARPIT_DELAY_MS:=10000}" "${TARPIT_MAX_CLIENTS:=4096}"
: "${DRY_RUN:=0}" "${DEDUPE_SECONDS:=900}" "${CATEGORIES:=18,22}" "${CURL_TIMEOUT:=10}"
: "${API_TOKEN:=}" "${ALLOWLIST:=}" "${ENDLESSH_ARGS:=}"
: "${REPORTED_FILE=reportedIps.txt}" # empty disables the audit file

for v in TARPIT_PORT TARPIT_DELAY_MS TARPIT_MAX_CLIENTS DEDUPE_SECONDS CURL_TIMEOUT; do
  [[ ${!v} =~ ^[0-9]+$ ]] || die "$v must be a number"
done
[[ $CATEGORIES =~ ^[0-9]+(,[0-9]+)*$ ]] || die "CATEGORIES must look like 18,22"
[[ $DRY_RUN == 0 || $DRY_RUN == 1 ]] || die "DRY_RUN must be 0 or 1"

ALLOWLIST=${ALLOWLIST,,}
# shellcheck disable=SC2086 # word splitting intended
for entry in ${ALLOWLIST//,/ }; do
  valid_ip "$entry" || die "ALLOWLIST takes plain ips, not '$entry' (no cidr ranges)"
done
ALLOWLIST=" ${ALLOWLIST//,/ } "

HDR_FILE=$(mktemp)
BODY_FILE=$(mktemp)
ENDLESSH_PID=""
# shellcheck disable=SC2317,SC2329 # runs via trap (old shellcheck says 2317, new 2329)
cleanup() {
  trap - EXIT
  [ -z "$ENDLESSH_PID" ] || kill "$ENDLESSH_PID" 2>/dev/null || true
  rm -f "$HDR_FILE" "$BODY_FILE"
}
trap 'exit 143' TERM
trap 'exit 130' INT
trap cleanup EXIT

if [ "$DRY_RUN" = 1 ]; then
  export -n API_TOKEN
  log "DRY_RUN=1: connections are logged, nothing is sent to AbuseIPDB"
else
  load_token
  [ -n "$API_TOKEN" ] || die "API_TOKEN is not set (https://www.abuseipdb.com/account/api), or set DRY_RUN=1 to try without"
  check_token
fi

declare -A SEEN=()
LAST_PRUNE=0

mark_seen() {
  SEEN[$1]=$SECONDS
  (( SECONDS - LAST_PRUNE >= 60 && ${#SEEN[@]} > 5000 )) || return 0
  LAST_PRUNE=$SECONDS
  local ip t
  for ip in "${!SEEN[@]}"; do
    t=${SEEN[$ip]}
    (( SECONDS - t < DEDUPE_SECONDS )) || unset "SEEN[$ip]"
  done
}

seen_recently() {
  local t=${SEEN[$1]:-}
  [ -n "$t" ] && (( SECONDS - t < DEDUPE_SECONDS ))
}

record_reported() {
  [ -n "$REPORTED_FILE" ] || return 0
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" >> "$REPORTED_FILE" \
    || log "warning: cannot write $REPORTED_FILE"
}

handle_accept() {
  local line=$1 rc=0
  extract_ip "$line" || { log "ignoring unparsable line: ${line:0:100}"; return 0; }
  if is_reserved "$IP"; then log "connect $IP -> skipped (private/reserved address)"; return 0; fi
  if is_allowlisted "$IP"; then log "connect $IP -> skipped (allowlisted)"; return 0; fi
  if seen_recently "$IP"; then log "connect $IP -> skipped (reported < ${DEDUPE_SECONDS}s ago)"; return 0; fi
  if [ "$DRY_RUN" = 1 ]; then log "connect $IP -> dry-run, not sent"; mark_seen "$IP"; return 0; fi
  if (( SECONDS < PAUSED_UNTIL )); then
    log "connect $IP -> skipped (reporting paused for $(( PAUSED_UNTIL - SECONDS ))s)"
    return 0
  fi
  report_ip "$IP" "$line" || rc=$?
  case $rc in
    0) record_reported "$IP"; mark_seen "$IP" ;;
    2) mark_seen "$IP" ;;
  esac
}

read -ra extra_args <<< "$ENDLESSH_ARGS"
log "starting endlessh on port $TARPIT_PORT (dry-run=$DRY_RUN, dedupe=${DEDUPE_SECONDS}s)"
# endlessh logs to stderr; merge it into the pipe. env -u keeps the token out of its environment
exec 3< <(exec env -u API_TOKEN endlessh -v -p "$TARPIT_PORT" -d "$TARPIT_DELAY_MS" \
  -m "$TARPIT_MAX_CLIENTS" "${extra_args[@]}" 2>&1)
ENDLESSH_PID=$!

while IFS= read -r -u 3 line; do
  case $line in
    *ACCEPT\ host=*) handle_accept "$line" ;;
    *CLOSE\ host=*) ;;
    *) log "endlessh: $line" ;; # startup info and errors like "address already in use"
  esac
done

code=0
wait "$ENDLESSH_PID" 2>/dev/null || code=$?
ENDLESSH_PID=""
log "endlessh exited (status $code), see messages above"
exit 1
