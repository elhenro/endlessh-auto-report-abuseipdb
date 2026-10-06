# shellcheck shell=bash
# shellcheck disable=SC2034 # EVENT_* are read only by this file
# append-only jsonl event log for dashboards; sourced by tarpitReporter.sh
# needs: EVENTS_DIR EVENTS_RETENTION_DAYS and lib/common.sh. an empty EVENTS_DIR disables it
#
# one file per utc day: $EVENTS_DIR/events-YYYY-MM-DD.jsonl, one object per line
#   {"t":1760000000,"e":"accept","ip":"1.2.3.4","a":"reported","q":2995}
#   {"t":1760000812,"e":"close","ip":"1.2.3.4","s":812.345,"b":4096}
# a = reported|dedup|dry|paused|failed|rejected, q = api quota left, s = seconds held, b = bytes sent
# values are validated numbers, enums or checked ips, so no json escaping is needed

EVENT_FILE=""
EVENT_DAY=""
EVENT_WARNED=0

events_init() {
  [ -n "$EVENTS_DIR" ] || return 0
  export TZ=UTC # %()T below and the daily file name follow utc
  # 755/644: a dashboard running as another uid reads these (attacker ips are public anyway)
  ( umask 022; mkdir -p "$EVENTS_DIR" ) 2>/dev/null \
    || { log "warning: cannot create $EVENTS_DIR, events disabled"; EVENTS_DIR=""; return 0; }
  log "writing events to $EVENTS_DIR (keeping ${EVENTS_RETENTION_DAYS} days)"
}

events_prune() {
  find "$EVENTS_DIR" -maxdepth 1 -name 'events-*.jsonl' -mtime "+$EVENTS_RETENTION_DAYS" -delete 2>/dev/null || true
}

# usage: emit_event <json members without braces or timestamp>
emit_event() {
  [ -n "$EVENTS_DIR" ] || return 0
  local now day
  printf -v now '%(%s)T' -1
  printf -v day '%(%Y-%m-%d)T' -1
  if [ "$day" != "$EVENT_DAY" ]; then
    EVENT_DAY=$day
    EVENT_FILE=$EVENTS_DIR/events-$day.jsonl
    ( umask 022; : >> "$EVENT_FILE" ) 2>/dev/null || true
    events_prune
  fi
  { printf '{"t":%s,%s}\n' "$now" "$1" >> "$EVENT_FILE"; } 2>/dev/null && return 0
  [ "$EVENT_WARNED" = 1 ] || { EVENT_WARNED=1; log "warning: cannot write $EVENT_FILE"; }
}

event_accept() { # ip action [quota]
  local q=""
  [[ ${3:-} =~ ^[0-9]+$ ]] && q=",\"q\":$3"
  emit_event "\"e\":\"accept\",\"ip\":\"$1\",\"a\":\"$2\"$q"
}

event_close() { # ip seconds bytes
  emit_event "\"e\":\"close\",\"ip\":\"$1\",\"s\":$2,\"b\":$3"
}
