#!/bin/bash
set -eu

# endlessh logs to stderr; merge with stdout so the pipe sees it
# grep --line-buffered ensures lines stream instead of buffering 4kB at a time
trap 'kill 0 2>/dev/null' EXIT INT TERM

endlessh -f endlessh-config-example.conf -v 2>&1 \
  | grep --line-buffered 'ACCEPT' \
  | while IFS= read -r line; do
      ./report.sh "$line" || true
    done
