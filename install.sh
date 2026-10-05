#!/usr/bin/env bash
# guided installer: docker (default) or systemd (--systemd). never touches your sshd config.
set -eu
cd "$(dirname "$0")"

MODE=docker BUILD=0 DRY=0 YES=0 PORT=22
USAGE="usage: ./install.sh [--systemd] [--build] [--dry-run] [--yes]
  --systemd   install on the host as a hardened systemd service (needs root)
  --build     build the docker image locally instead of pulling it
  --dry-run   log visitors but report nothing (no API token needed)
  --yes       no prompts; needs API_TOKEN in the environment unless --dry-run"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

ask() { # prompt default -> $REPLY
  if [ "$YES" = 1 ]; then REPLY=$2; return; fi
  read -r -p "$1 [$2]: " REPLY
  REPLY=${REPLY:-$2}
}

# http status of an authenticated call; the token travels via stdin, not argv. 000 = unreachable
token_status() {
  printf 'header = "Key: %s"\n' "$1" | curl -sS -o /dev/null -w '%{http_code}' \
    --max-time 10 --proto '=https' -K - -G --data-urlencode ipAddress=127.0.0.1 \
    -H 'Accept: application/json' https://api.abuseipdb.com/api/v2/check 2>/dev/null || true
}

get_token() {
  local tries=0 status
  while :; do
    if [ -z "${API_TOKEN:-}" ]; then
      [ "$YES" = 0 ] || die "set API_TOKEN, or pass --dry-run"
      read -r -s -p "AbuseIPDB API key (https://www.abuseipdb.com/account/api): " API_TOKEN; echo
    fi
    status=$(token_status "$API_TOKEN")
    case $status in
      200) say "token ok"; return ;;
      401|403)
        [ "$YES" = 0 ] || die "AbuseIPDB rejected the token (HTTP $status)"
        say "AbuseIPDB rejected that token (HTTP $status)"
        API_TOKEN=""; tries=$((tries + 1))
        [ "$tries" -lt 3 ] || die "giving up" ;;
      *) say "warning: could not validate the token (HTTP $status), continuing"; return ;;
    esac
  done
}

port_busy() {
  if command -v ss >/dev/null; then [ -n "$(ss -ltnH "sport = :$1" 2>/dev/null)" ]
  elif command -v lsof >/dev/null; then lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1
  else (exec 3<>/dev/tcp/127.0.0.1/"$1") 2>/dev/null; fi
}

# sets PORT; a port held by our own running install does not count as busy
choose_port() { # default
  PORT=${HOST_PORT:-${TARPIT_PORT:-$1}}
  if port_busy "$PORT" && ! own_service_running; then
    say "port $PORT is in use (on :22 that is probably your real sshd)."
    say "try the tarpit on 2222 first; move sshd, then re-run with HOST_PORT=22."
    ask "port to use" 2222; PORT=$REPLY
    ! port_busy "$PORT" || die "port $PORT is busy too"
  fi
}

own_service_running() {
  if [ "$MODE" = systemd ]; then systemctl is-active --quiet endlessh-reporter 2>/dev/null
  else [ -n "$(docker compose ps -q 2>/dev/null)" ]; fi
}

# connect once, then look for the reporter's "connect" log line
self_test() { # log command...
  say "testing the tarpit on port $PORT ..."
  for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
    (exec 3<>/dev/tcp/127.0.0.1/"$PORT") 2>/dev/null && break
    sleep 1
  done
  sleep 1
  if "$@" 2>&1 | grep -q 'tarpit: connect '; then
    say "OK: the tarpit accepted a test connection and the reporter saw it."
  else
    say "WARNING: no connection showed up in the logs. check: $*"
  fi
}

write_env() {
  if [ -f .env ]; then
    say "keeping existing .env (delete it to reconfigure)"
    PORT=$(sed -n 's/^HOST_PORT=//p' .env | tail -1)
    PORT=${PORT:-22}
    return
  fi
  choose_port 22
  [ "$DRY" = 1 ] || get_token
  ask "your own public ips to never report (comma separated, optional)" ""
  ( umask 077
    printf 'API_TOKEN=%s\nHOST_PORT=%s\nDRY_RUN=%s\nALLOWLIST=%s\n' "${API_TOKEN:-}" "$PORT" "$DRY" "$REPLY" > .env )
}

install_docker() {
  command -v docker >/dev/null || die "docker not found: https://docs.docker.com/engine/install/"
  docker compose version >/dev/null 2>&1 || die "docker compose plugin missing"
  docker info >/dev/null 2>&1 || die "cannot reach the docker daemon (permissions? try sudo)"
  write_env
  if [ "$BUILD" = 1 ]; then docker compose build
  else docker compose pull || { say "pull failed, building locally"; docker compose build; }; fi
  docker compose up -d
  self_test docker compose logs --since 1m
  say "logs:    docker compose logs -f"
  say "stop:    docker compose down"
}

next_steps() {
  if [ "$PORT" = 22 ]; then
    say "tarpit is live on :22. check that your real sshd answers on its new port from a second terminal."
  else
    say "tarpit is running on :$PORT. bots mostly hit :22, so to go live:"
    say "  1. set 'Port <n>' in sshd_config (n in 1024-65535), run 'sshd -t', open <n> in your firewall/cloud security group"
    say "  2. reload sshd and confirm 'ssh -p <n> host' works from a second terminal BEFORE closing this one"
    say "  3. re-run with HOST_PORT=22 (delete .env first, or edit HOST_PORT in it)"
  fi
}

main() {
  while [ $# -gt 0 ]; do
    case $1 in
      --systemd) MODE=systemd ;;
      --build) BUILD=1 ;;
      --dry-run) DRY=1 ;;
      --yes|-y) YES=1 ;;
      -h|--help) say "$USAGE"; exit 0 ;;
      *) say "$USAGE" >&2; exit 1 ;;
    esac
    shift
  done
  if [ "$MODE" = systemd ]; then
    # shellcheck source=systemd/install.sh
    . ./systemd/install.sh
    install_systemd
  else
    install_docker
  fi
  next_steps
}

# run only when executed, so tests can source the helpers
if [ "${BASH_SOURCE[0]}" = "$0" ]; then main "$@"; fi
