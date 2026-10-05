# shellcheck shell=bash
# shellcheck disable=SC2034 # PAUSED_UNTIL is read by tarpitReporter.sh
# AbuseIPDB api client; sourced by tarpitReporter.sh
# needs: API_TOKEN CATEGORIES CURL_TIMEOUT HDR_FILE BODY_FILE and lib/common.sh

readonly API_BASE=https://api.abuseipdb.com/api/v2
PAUSED_UNTIL=0
HTTP_CODE=000

load_token() {
  if [ -z "$API_TOKEN" ] && [ -n "${API_TOKEN_FILE:-}" ]; then
    API_TOKEN=$(cat "$API_TOKEN_FILE") || die "cannot read API_TOKEN_FILE=$API_TOKEN_FILE"
  elif [ -z "$API_TOKEN" ] && [ -n "${CREDENTIALS_DIRECTORY:-}" ]; then
    # systemd LoadCredential; needs a kernel with tmpfs ACLs when running as a non-root user
    API_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/api_token") \
      || die "cannot read $CREDENTIALS_DIRECTORY/api_token; put API_TOKEN=... in the unit's env file instead"
  fi
  API_TOKEN=${API_TOKEN//[[:space:]]/}
  [ "$API_TOKEN" = replace-me-with-abuseipdb-token ] && API_TOKEN=""
  export -n API_TOKEN
  return 0
}

# usage: api_call <path> [curl args]; sets HTTP_CODE (000 = no answer), headers in HDR_FILE
# token goes in via a config fd, never argv, so it is invisible in /proc/<pid>/cmdline
api_call() {
  local path=$1; shift
  HTTP_CODE=$(curl -sS -o "$BODY_FILE" -D "$HDR_FILE" -w '%{http_code}' \
    --connect-timeout 5 --max-time "$CURL_TIMEOUT" --proto '=https' --proto-redir '=https' --tlsv1.2 \
    -K <(printf 'header = "Key: %s"\n' "$API_TOKEN") -H 'Accept: application/json' \
    "$@" "$API_BASE/$path" 2>/dev/null) || HTTP_CODE=000
}

header_value() {
  awk -v h="$1:" 'tolower($1) == h { v = $2 } END { sub(/\r$/, "", v); print v }' "$HDR_FILE"
}

pause_for() { # seconds fallback reason
  local secs=${1:-}
  [[ $secs =~ ^[0-9]+$ ]] || secs=$2
  (( secs > 86400 )) && secs=86400
  PAUSED_UNTIL=$(( SECONDS + secs ))
  log "pausing reports for ${secs}s: $3"
}

# a rejected token is fatal at startup: better a loud crash than silent no-op reporting
check_token() {
  api_call check -G --data-urlencode ipAddress=127.0.0.1
  case $HTTP_CODE in
    200) log "AbuseIPDB token ok" ;;
    401|403) die "AbuseIPDB rejected the API token (HTTP $HTTP_CODE)" ;;
    000) log "warning: cannot reach AbuseIPDB right now, continuing" ;;
    *) log "warning: token check returned HTTP $HTTP_CODE, continuing" ;;
  esac
}

# returns 0 reported, 1 failed (retry on a later connect), 2 rejected by api (do not retry)
report_ip() { # ip message
  api_call report --data-urlencode "ip=$1" -d "categories=$CATEGORIES" \
    --data-urlencode "comment=SSH login attempts (endlessh): $2"
  case $HTTP_CODE in
    200) log "connect $1 -> reported (quota left: $(header_value x-ratelimit-remaining))" ;;
    422) log "connect $1 -> rejected by AbuseIPDB: $(head -c 200 "$BODY_FILE")"; return 2 ;;
    429) pause_for "$(header_value retry-after)" 3600 "daily quota exhausted"; return 1 ;;
    401|403) pause_for 3600 3600 "token rejected (HTTP $HTTP_CODE)"; return 1 ;;
    *) log "connect $1 -> FAILED (HTTP $HTTP_CODE)"; pause_for 30 30 "api unavailable"; return 1 ;;
  esac
}
