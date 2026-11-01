#!/bin/sh
# runs from its own directory, so the audit file lands beside it
cd "$(dirname "$0")" || exit 1
: "${REPORTED_FILE=reportedIps.txt}"
export REPORTED_FILE
[ -x target/release/endlessh-reporter ] || {
  echo "tarpit: error: build it first: cargo build --release --locked" >&2
  exit 78
}
exec target/release/endlessh-reporter "$@"
