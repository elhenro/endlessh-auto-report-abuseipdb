#!/bin/bash
read message
APIKEY="<api-token>"

IP=$(echo ${message} | grep -o -P '(?<=host=::ffff:).*(?= port)')
comment="SSH login attempts (endlessh): ${message}"
echo "reporting IP:'${IP}' - ${comment}"

if [[ $(bash cache.sh 900 "echo $IP") = 'true' ]]; then
    echo "${IP} is still cached!"
else
  curl -s https://api.abuseipdb.com/api/v2/report \
      --data-urlencode "ip=${IP}" \
        -d categories=18,22 \
        --data-urlencode "comment=${comment}" \
            -H "Key: ${APIKEY}" \
              -H "Accept: application/json" | jq
  echo "${IP}" >> reportedIps.txt
fi
