# shellcheck shell=bash
# bare-metal install path; sourced by install.sh (uses its helpers)

INSTALL_DIR=/opt/endlessh-reporter
CONF_DIR=/etc/endlessh-reporter

# a prebuilt binary next to install.sh, else the cargo build output, else build now
find_binary() {
  BIN_NAME=endlessh-reporter
  for BIN in "./$BIN_NAME" "target/release/$BIN_NAME"; do [ -x "$BIN" ] && return 0; done
  command -v cargo >/dev/null || die "no $BIN_NAME binary: run 'cargo build --release --locked' as your user first (https://rustup.rs), then re-run with sudo"
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
    return
  fi
  choose_port 22
  [ "$DRY" = 1 ] || get_token
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
  # an older install binds the port itself; release it before the socket unit takes over
  systemctl stop endlessh-reporter 2>/dev/null || true
  install -d -m 755 "$INSTALL_DIR"
  rm -rf "${INSTALL_DIR:?}/lib" "${INSTALL_DIR:?}/tarpitReporter.sh" # leftovers of the old bash reporter
  install -m 755 "$BIN" "$INSTALL_DIR/$BIN_NAME"
  install -m 644 systemd/endlessh-reporter.service /etc/systemd/system/
  sed "s/^ListenStream=.*/ListenStream=$PORT/" systemd/endlessh-reporter.socket > /etc/systemd/system/endlessh-reporter.socket
  chmod 644 /etc/systemd/system/endlessh-reporter.socket
  systemctl daemon-reload
  systemctl enable endlessh-reporter >/dev/null 2>&1
  # a bad config can leave both failed: every visitor triggers a start, until the start limit stops the socket too
  systemctl reset-failed endlessh-reporter.socket endlessh-reporter 2>/dev/null || true
  systemctl restart endlessh-reporter.socket
  # started right away, not on the first visitor, so a bad token shows up now
  systemctl restart endlessh-reporter
  self_test journalctl -u endlessh-reporter --since -1min --no-pager
  say "logs:    journalctl -fu endlessh-reporter"
  say "config:  $CONF_DIR/env (then: systemctl restart endlessh-reporter; for TARPIT_PORT re-run this installer)"
}
