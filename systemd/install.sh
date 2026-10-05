# shellcheck shell=bash
# bare-metal install path; sourced by install.sh (uses its helpers)

INSTALL_DIR=/opt/endlessh-reporter
CONF_DIR=/etc/endlessh-reporter

ensure_packages() {
  local missing="" bin
  for bin in endlessh curl; do command -v "$bin" >/dev/null || missing="$missing $bin"; done
  [ -n "$missing" ] || return 0
  command -v apt-get >/dev/null || die "missing:$missing - install them with your package manager and re-run"
  ask "install$missing via apt-get? (y/n)" y
  [ "$REPLY" = y ] || die "need$missing"
  # shellcheck disable=SC2086 # word splitting intended
  apt-get update -qq && apt-get install -y -qq --no-install-recommends $missing
  # the debian package auto-starts its own unit on :2222; we run our own instead
  case $missing in *endlessh*) systemctl disable --now endlessh >/dev/null 2>&1 || true ;; esac
}

write_config() {
  install -d -m 755 "$CONF_DIR"
  if [ -f "$CONF_DIR/env" ]; then
    say "keeping existing $CONF_DIR/env and api_token (delete them to reconfigure)"
    PORT=$(sed -n 's/^TARPIT_PORT=//p' "$CONF_DIR/env" | tail -1)
    PORT=${PORT:-22}
    return
  fi
  choose_port 22
  [ "$DRY" = 1 ] || get_token
  ask "your own public ips to never report (comma separated, optional)" ""
  printf 'TARPIT_PORT=%s\nDRY_RUN=%s\nALLOWLIST=%s\n' "$PORT" "$DRY" "$REPLY" > "$CONF_DIR/env"
  # the credential file must exist even for a dry run, or the unit refuses to start
  ( umask 077; printf '%s' "${API_TOKEN:-}" > "$CONF_DIR/api_token" )
}

install_systemd() {
  [ "$(id -u)" = 0 ] || die "run as root: sudo ./install.sh --systemd"
  command -v systemctl >/dev/null || die "systemd not found - use the docker install"
  ensure_packages
  write_config
  install -d -m 755 "$INSTALL_DIR/lib"
  install -m 755 tarpitReporter.sh "$INSTALL_DIR/"
  install -m 644 lib/*.sh "$INSTALL_DIR/lib/"
  install -m 644 systemd/endlessh-reporter.service /etc/systemd/system/
  systemctl daemon-reload
  systemctl enable endlessh-reporter >/dev/null 2>&1
  systemctl restart endlessh-reporter
  self_test journalctl -u endlessh-reporter --since -1min --no-pager
  say "logs:    journalctl -fu endlessh-reporter"
  say "config:  $CONF_DIR/env (then: systemctl restart endlessh-reporter)"
}
