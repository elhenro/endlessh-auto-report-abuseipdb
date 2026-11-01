# shellcheck shell=bash
# which port to use and whether it is safe to take; sourced by install.sh (uses its helpers)

port_busy() {
  if command -v ss >/dev/null; then [ -n "$(ss -ltnH "sport = :$1" 2>/dev/null)" ]
  elif command -v lsof >/dev/null; then lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1
  else (exec 3<>/dev/tcp/127.0.0.1/"$1") 2>/dev/null; fi
}

# our running install already listens on exactly this port. running on another one does not count
own_port() {
  if [ "$MODE" = systemd ]; then
    systemctl is-active --quiet endlessh-reporter.socket 2>/dev/null &&
      grep -qxE "ListenStream=(.*:)?$1" "$SOCKET" 2>/dev/null
  else
    docker compose port endlessh 2222 2>/dev/null | grep -q ":$1\$"
  fi
}

# an enabled sshd takes this port at its next start, running or not. sshd -T is the authority, but it
# needs root and /run/sshd, which goes away with sshd; else the config files, where no Port line means 22
sshd_wants() {
  command -v systemctl >/dev/null || return 1
  local unit ports enabled=0
  for unit in ssh sshd ssh.socket sshd.socket; do
    systemctl is-enabled --quiet "$unit" 2>/dev/null && enabled=1
  done
  [ "$enabled" = 1 ] || return 1
  # socket-activated sshd: the socket unit owns the port, and sshd_config's Port is not what it listens
  # on. an install that moved sshd to make room for the tarpit does exactly this, so read Listen first
  for unit in sshd.socket ssh.socket; do
    systemctl is-enabled --quiet "$unit" 2>/dev/null || continue
    ports=$(systemctl show -p Listen --value "$unit" 2>/dev/null | grep -oE ':[0-9]+' | tr -d :)
    if [ -n "$ports" ]; then
      printf '%s\n' "$ports" | grep -qx "$1"
      return
    fi
  done
  ports=""
  if command -v sshd >/dev/null; then
    ports=$(sshd -T 2>/dev/null | awk 'tolower($1) == "port" { print $2 }')
  fi
  [ -n "$ports" ] || ports=$(cat /etc/ssh/sshd_config /etc/ssh/sshd_config.d/*.conf 2>/dev/null | awk 'tolower($1) == "port" { print $2 }')
  printf '%s\n' "${ports:-22}" | grep -qx "$1"
}

port_taken() { # port, with or without a bind address in front
  set -- "${1##*:}"
  sshd_wants "$1" || { port_busy "$1" && ! own_port "$1"; }
}

# sets PORT
choose_port() { # default
  PORT=${HOST_PORT:-${TARPIT_PORT:-$1}}
  if port_taken "$PORT"; then
    say "port $PORT is in use (on :22 that is probably your real sshd)."
    say "try the tarpit on 2222 first; move sshd, then re-run with HOST_PORT=22."
    ask "port to use" 2222; PORT=$REPLY
    ! port_taken "$PORT" || die "port $PORT is busy too"
  fi
}

# on every install: a kept config skips choose_port, and a tarpit left set to take the port
# of a running sshd looks harmless until the next boot, which sshd then does not survive
require_port() {
  if port_taken "$PORT"; then
    die "port $PORT is held by something else or sshd is configured for it. not installing: see 'Before you use port 22' in the readme"
  fi
}

# browsers refuse 22 but not other ports, so reports wait until the operator moves the tarpit there on purpose
hold_reports() {
  [ "${DRY_RUN:-}" != 0 ] || return 0 # asked for by name
  if [ "${PORT##*:}" != 22 ] && [ "$DRY" = 0 ]; then
    DRY=1
    say "port $PORT is not 22, so starting with DRY_RUN=1: visitors are logged, nothing gets reported yet."
  fi
}
