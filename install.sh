#!/usr/bin/env bash
# guided installer: docker (default) or systemd (--systemd). never touches your sshd config.
set -eu
cd "$(dirname "$0")"
# shellcheck source=install-ports.sh
. ./install-ports.sh

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

# connect, then look for the reporter's "connect" log line, which comes on the tarpit's first tick
# for that client. retries, because docker's proxy accepts on the published port while the binary
# is still checking the token and not listening yet
self_test() { # log command...
  say "testing the tarpit on port $PORT ..."
  for _ in 1 2 3 4 5 6 7 8; do
    (exec 3<>/dev/tcp/127.0.0.1/"${PORT##*:}") 2>/dev/null || true
    sleep 3
    if "$@" 2>&1 | grep -q 'tarpit: connect '; then
      say "OK: the tarpit accepted a test connection and the reporter saw it."
      return
    fi
  done
  say "WARNING: no connection showed up in the logs. check: $*"
}

# a kernel booted with ipv6 off cannot create that network
compose_files() {
  printf 'docker-compose.yml:docker-compose.secrets.yml'
  [ ! -d /proc/sys/net/ipv6 ] || printf ':docker-compose.ipv6.yml'
}

# the container's uid must read the key: the file is 0644, the 0700 directory keeps other host users out
write_token() {
  ( umask 077; mkdir -p secrets )
  chmod 700 secrets
  ( umask 022; printf '%s' "${API_TOKEN:-}" > secrets/api_token )
}

write_env() {
  if [ -f .env ]; then
    say "keeping existing .env (delete it to reconfigure)"
    PORT=$(sed -n 's/^HOST_PORT=//p' .env | tail -1 | tr -d "\"'")
    PORT=${PORT:-22}
    DRY=$(sed -n 's/^DRY_RUN=//p' .env | tail -1 | tr -d "\"'")
    DRY=${DRY:-0}
    if grep -q '^API_TOKEN=.' .env; then
      say "note: API_TOKEN in .env keeps working but shows up in docker inspect, see 'Upgrading from the bash version' in the readme"
    fi
    # only a .env that turned on the secrets file needs it to exist
    if grep -q '^COMPOSE_FILE=.*secrets' .env && [ ! -f secrets/api_token ]; then
      API_TOKEN="" write_token
    fi
    return
  fi
  choose_port 22
  [ "$DRY" = 1 ] || get_token
  hold_reports
  ask "your own public ips to never report (comma separated, optional)" ""
  ( umask 077
    printf 'COMPOSE_FILE=%s\nHOST_PORT=%s\nDRY_RUN=%s\nALLOWLIST=%s\n' "$(compose_files)" "$PORT" "$DRY" "$REPLY" > .env )
  write_token
}

install_docker() {
  command -v docker >/dev/null || die "docker not found: https://docs.docker.com/engine/install/"
  docker compose version >/dev/null 2>&1 || die "docker compose plugin missing"
  docker info >/dev/null 2>&1 || die "cannot reach the docker daemon (permissions? try sudo)"
  write_env
  require_port
  if [ "$BUILD" = 1 ]; then docker compose build
  else docker compose pull || { say "pull failed, building locally"; docker compose build; }; fi
  # a container that failed to start stays around, and its restart policy can try again at next boot
  if ! docker compose up -d; then
    docker compose down >/dev/null 2>&1 || true
    die "could not start the tarpit, see above"
  fi
  self_test docker compose logs --since 1m
  say "logs:    docker compose logs -f"
  say "stop:    docker compose down"
}

next_steps() {
  if [ "${PORT##*:}" = 22 ]; then
    say "tarpit is live on :22. check that your real sshd answers on its new port from a second terminal."
  else
    say "tarpit is running on :$PORT. bots mostly hit :22, so to go live:"
    say "  1. set 'Port <n>' in sshd_config (n in 1024-65535), run 'sshd -t', open <n> in your firewall/cloud security group"
    say "  2. reload sshd and confirm 'ssh -p <n> host' works from a second terminal BEFORE closing this one"
    if [ "$MODE" = systemd ]; then
      say "  3. set TARPIT_PORT=22 and DRY_RUN=0 in /etc/endlessh-reporter/env and re-run this installer"
    else
      say "  3. set HOST_PORT=22 and DRY_RUN=0 in .env and re-run this installer"
    fi
    if [ "$DRY" = 0 ]; then
      say "warning: DRY_RUN=0 on :$PORT. browsers only refuse 22, and a web page using dns rebinding can make them"
      say "open with SSH- here and get its visitors reported. set DRY_RUN=1 until the tarpit is on 22."
    fi
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
