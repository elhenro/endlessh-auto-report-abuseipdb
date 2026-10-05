# shellcheck shell=bash
# shared helpers; sourced by tarpitReporter.sh

log() { printf 'tarpit: %s\n' "$*"; }

# config errors exit 78 (EX_CONFIG) so systemd does not restart-loop on them
die() { printf 'tarpit: error: %s\n' "$*" >&2; exit 78; }
