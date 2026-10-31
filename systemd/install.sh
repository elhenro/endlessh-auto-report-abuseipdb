# shellcheck shell=bash
# bare-metal install path; sourced by install.sh (uses its helpers)

INSTALL_DIR=/opt/endlessh-reporter
CONF_DIR=/etc/endlessh-reporter
SOCKET=/etc/systemd/system/endlessh-reporter.socket

# a prebuilt binary next to install.sh, else the cargo build output, else build now
find_binary() {
  BIN_NAME=endlessh-reporter
  for BIN in "./$BIN_NAME" "target/release/$BIN_NAME"; do
    [ -x "$BIN" ] || continue
    # older than the checkout: installing it would silently skip whatever changed, security fixes included
    [ -z "$(find src Cargo.lock Cargo.toml -newer "$BIN" 2>/dev/null | head -1)" ] && return 0
    say "ignoring $BIN: it is older than the source"
  done
  command -v cargo >/dev/null || die "no current $BIN_NAME binary: run 'cargo build --release --locked' as your user first (https://rustup.rs), then re-run with sudo"
  say "building $BIN_NAME ..."
  cargo build --release --locked
  BIN=target/release/$BIN_NAME
}

write_config() {
  install -d -m 700 "$CONF_DIR"
  if [ -f "$CONF_DIR/env" ]; then
    say "keeping existing $CONF_DIR/env and api_token (delete them to reconfigure)"
    chmod 600 "$CONF_DIR/env" # older installs left it readable by all, and it lists your own ips
    PORT=$(sed -n 's/^TARPIT_PORT=//p' "$CONF_DIR/env" | tail -1)
    PORT=${PORT:-22}
    DRY=$(sed -n 's/^DRY_RUN=//p' "$CONF_DIR/env" | tail -1)
    DRY=${DRY:-0}
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
  install -m 644 systemd/endlessh-reporter.service /etc/systemd/system/
  sed "s/^ListenStream=.*/ListenStream=$PORT/" systemd/endlessh-reporter.socket > "$SOCKET"
  chmod 644 "$SOCKET"
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
  say "config:  $CONF_DIR/env (then: systemctl restart endlessh-reporter; for TARPIT_PORT or DRY_RUN re-run this installer)"
  # personal data this version no longer touches. whose call it is to delete: the operator's
  for old in "/var/lib/private/$BIN_NAME/reportedIps.txt" "/var/lib/$BIN_NAME/reportedIps.txt"; do
    if [ -f "$old" ]; then
      say "note: $old from the old reporter is no longer used or pruned. delete it if you do not need it"
      break
    fi
  done
}
