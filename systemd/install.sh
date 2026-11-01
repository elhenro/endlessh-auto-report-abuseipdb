# shellcheck shell=bash
# bare-metal install path; sourced by install.sh (uses its helpers)

INSTALL_DIR=/opt/endlessh-reporter
CONF_DIR=/etc/endlessh-reporter
SOCKET=/etc/systemd/system/endlessh-reporter.socket

# a host without rust gets the release build: the old version needed no toolchain here either
download_binary() {
  local url=https://github.com/elhenro/endlessh-auto-report-abuseipdb/releases/latest/download
  local name dir
  name="$BIN_NAME-$(uname -m)"
  ask "no rust toolchain here. download $name from the latest release? (y/n)" y
  [ "$REPLY" = y ] || return 1
  dir=$(mktemp -d)
  if curl -fsSL --proto '=https' --tlsv1.2 --max-time 120 -o "$dir/$name" "$url/$name" &&
    curl -fsSL --proto '=https' --tlsv1.2 --max-time 30 -o "$dir/SHA256SUMS" "$url/SHA256SUMS" &&
    ( cd "$dir" && grep " $name\$" SHA256SUMS | sha256sum -c --quiet - ); then
    chmod 755 "$dir/$name"
    BIN=$dir/$name BIN_TMP=$dir
    return 0
  fi
  rm -rf "$dir"
  return 1
}

# a prebuilt binary next to install.sh, else the cargo build output, else build now, else the release
find_binary() {
  BIN_NAME=endlessh-reporter
  for BIN in "./$BIN_NAME" "target/release/$BIN_NAME"; do
    [ -x "$BIN" ] || continue
    # older than the checkout: installing it would silently skip whatever changed, security fixes included
    [ -z "$(find src Cargo.lock Cargo.toml -newer "$BIN" 2>/dev/null | head -1)" ] && return 0
    say "ignoring $BIN: it is older than the source"
  done
  if ! command -v cargo >/dev/null; then
    download_binary || die "no current $BIN_NAME binary: run 'cargo build --release --locked' as your user first (https://rustup.rs), then re-run with sudo"
    return 0
  fi
  say "building $BIN_NAME ..."
  cargo build --release --locked
  BIN=target/release/$BIN_NAME
}

# endlessh's flags in ENDLESSH_ARGS: the socket unit binds now, so the ones about binding apply here
read_bind_flags() {
  # shellcheck disable=SC2046 # word splitting intended
  set -- $(sed -n 's/^ENDLESSH_ARGS=//p' "$CONF_DIR/env" | tail -1 | tr -d "\"'")
  while [ $# -gt 0 ]; do
    case $1 in
      -4) FAMILY=4 ;;
      -6) FAMILY=6 ;;
      -p) PORT=${2:-$PORT}; [ $# -lt 2 ] || shift ;;
      -p*) PORT=${1#-p} ;;
    esac
    shift
  done
}

write_socket() {
  local listen=$PORT only=both
  case ${FAMILY:-} in
    4) listen=0.0.0.0:$PORT ;;
    6) listen="[::]:$PORT" only=ipv6-only ;;
  esac
  sed -e "s/^ListenStream=.*/ListenStream=$listen/" -e "s/^BindIPv6Only=.*/BindIPv6Only=$only/" \
    systemd/endlessh-reporter.socket > "$SOCKET"
  chmod 644 "$SOCKET"
}

write_config() {
  install -d -m 700 "$CONF_DIR"
  if [ -f "$CONF_DIR/env" ]; then
    say "keeping existing $CONF_DIR/env and api_token (delete them to reconfigure)"
    chmod 600 "$CONF_DIR/env" # older installs left it readable by all, and it lists your own ips
    PORT=$(sed -n 's/^TARPIT_PORT=//p' "$CONF_DIR/env" | tail -1 | tr -d "\"'")
    PORT=${PORT:-2222} # what the service binds without the line
    DRY=$(sed -n 's/^DRY_RUN=//p' "$CONF_DIR/env" | tail -1 | tr -d "\"'")
    DRY=${DRY:-0}
    read_bind_flags
    return
  fi
  choose_port 22
  [ "$DRY" = 1 ] || get_token
  hold_reports
  ask "your own public ips to never report (comma separated, optional)" ""
  ( umask 077
    printf 'TARPIT_PORT=%s\nDRY_RUN=%s\nALLOWLIST=%s\n' "$PORT" "$DRY" "$REPLY" > "$CONF_DIR/env"
    # the credential file must exist even for a dry run, or the unit refuses to start
    printf '%s' "${API_TOKEN:-}" > "$CONF_DIR/api_token" )
}

install_systemd() {
  [ "$(id -u)" = 0 ] || die "run as root: sudo ./install.sh --systemd"
  command -v systemctl >/dev/null || die "systemd not found - use the docker install"
  find_binary
  write_config
  case $PORT in ''|*[!0-9]*) die "port must be a number, got '$PORT'" ;; esac
  # an older install binds the port itself, without a socket unit: release it, or it looks like someone else
  [ -f "$SOCKET" ] || systemctl stop endlessh-reporter 2>/dev/null || true
  require_port
  systemctl stop endlessh-reporter 2>/dev/null || true
  install -d -m 755 "$INSTALL_DIR"
  rm -rf "${INSTALL_DIR:?}/lib" "${INSTALL_DIR:?}/tarpitReporter.sh" # leftovers of the old bash reporter
  install -m 755 "$BIN" "$INSTALL_DIR/$BIN_NAME"
  [ -z "${BIN_TMP:-}" ] || rm -rf "$BIN_TMP"
  install -m 644 systemd/endlessh-reporter.service /etc/systemd/system/
  write_socket
  systemctl daemon-reload
  # a bad config can leave both failed: every visitor triggers a start, until the start limit stops the socket too
  systemctl reset-failed endlessh-reporter.socket endlessh-reporter 2>/dev/null || true
  # enabled only once it came up: at the next boot an enabled socket gets its port before sshd does
  if ! systemctl restart endlessh-reporter.socket; then
    systemctl disable endlessh-reporter >/dev/null 2>&1 || true
    die "could not listen on port $PORT, the tarpit stays off: journalctl -u endlessh-reporter.socket"
  fi
  systemctl enable endlessh-reporter >/dev/null 2>&1
  # started right away, not on the first visitor, so a bad token shows up now
  systemctl restart endlessh-reporter
  self_test journalctl -u endlessh-reporter --since -1min --no-pager
  say "logs:    journalctl -fu endlessh-reporter"
  say "config:  $CONF_DIR/env (then: systemctl restart endlessh-reporter; for TARPIT_PORT or a -4/-6/-p in ENDLESSH_ARGS re-run this installer)"
}
