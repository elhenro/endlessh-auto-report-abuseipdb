#!/bin/sh
PROG="$(basename $0)"
DIR="${HOME}/.cache/${PROG}"
mkdir -p "${DIR}"
EXPIRY=900 # default to 15 minutes
[ "$1" -eq "$1" ] 2>/dev/null && EXPIRY=$1 && shift
CMD="$@"
HASH=$(echo "$CMD" | md5sum | awk '{print $1}')
CACHE="$DIR/$HASH"

if test -f "${CACHE}" && [ $(expr $(date +%s) - $(date -r "$CACHE" +%s)) -le $EXPIRY ]; then
  echo 'true'
else
  echo 'false'
  test -f "${CACHE}" && [ $(expr $(date +%s) - $(date -r "$CACHE" +%s)) -le $EXPIRY ] || eval "$CMD" > "${CACHE}"
fi
