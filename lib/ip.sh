# shellcheck shell=bash
# ip parsing and filtering; sourced by tarpitReporter.sh

valid_ip() {
  local ip=$1 octet
  if [[ $ip =~ ^([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})$ ]]; then
    for octet in "${BASH_REMATCH[@]:1}"; do
      (( 10#$octet <= 255 )) || return 1
    done
    return 0
  fi
  [[ $ip =~ ^[0-9a-f:.]{2,45}$ && $ip == *:*:* ]]
}

# sets IP from an endlessh "ACCEPT host=<ip> port=<n> ..." line; fails unless it is a plain ip
extract_ip() {
  [[ $1 =~ host=([^[:space:]]+)[[:space:]]port= ]] || return 1
  IP=${BASH_REMATCH[1]#::ffff:}
  IP=${IP,,}
  valid_ip "$IP"
}

# loopback, private, link-local, cgnat, multicast/reserved: pointless or invalid to report
is_reserved() {
  if [[ $1 == *:* ]]; then
    [[ $1 =~ ^(::1?$|fe[89ab][0-9a-f]:|f[cd][0-9a-f]{2}:|ff[0-9a-f]{2}:) ]]
  else
    [[ $1 =~ ^(0\.|10\.|127\.|169\.254\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.|100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.|(22[4-9]|2[3-5][0-9])\.) ]]
  fi
}

# ALLOWLIST is normalized by the caller to " ip ip ip "
is_allowlisted() { [[ $ALLOWLIST == *" $1 "* ]]; }
