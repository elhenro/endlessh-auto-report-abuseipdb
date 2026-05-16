#!/bin/bash
set -eu

message="${1:-}"
[ -z "$message" ] && exit 0

APIKEY="${API_TOKEN:?API_TOKEN env var required}"

# strip the ::ffff: IPv4-in-IPv6 prefix endlessh adds; works for plain v6 too
IP=$(printf '%s' "$message" | grep -oP '(?<=host=)\S+(?= port)' | sed 's/^::ffff://')
[ -z "$IP" ] && { echo "report.sh: no IP in: $message" >&2; exit 0; }

if [ "$(./cache.sh 900 "echo $IP")" = 'true' ]; then
  echo "report.sh: cached, skip $IP"
  exit 0
fi

comment="SSH login attempts (endlessh): ${message}"
echo "report.sh: reporting $IP"

curl -fsS https://api.abuseipdb.com/api/v2/report \
    --data-urlencode "ip=${IP}" \
    -d categories=18,22 \
    --data-urlencode "comment=${comment}" \
    -H "Key: ${APIKEY}" \
    -H "Accept: application/json" \
    || echo "report.sh: abuseipdb call failed for $IP" >&2
echo

echo "${IP}" >> reportedIps.txt
