#!/bin/sh
# the old entry point, for whatever still starts the bash version by this name. runs from
# its own directory and keeps its audit file there, as that version did
cd "$(dirname "$0")" || exit 1
: "${REPORTED_FILE=reportedIps.txt}"
export REPORTED_FILE
[ -x target/release/endlessh-reporter ] || {
  echo "tarpit: error: build it first: cargo build --release --locked" >&2
  exit 78
}
exec target/release/endlessh-reporter "$@"
