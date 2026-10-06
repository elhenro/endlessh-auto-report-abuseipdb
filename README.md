# endlessh auto-report to AbuseIPDB

[![ci](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/ci.yml/badge.svg)](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/ci.yml)
[![image](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/docker.yml/badge.svg)](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/docker.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

SSH tarpit ([endlessh](https://github.com/skeeto/endlessh)) that stalls bots on port 22 and reports their IPs to [AbuseIPDB](https://www.abuseipdb.com/). Each IP is reported at most once per 15 minutes.

## Quickstart

You need a free [AbuseIPDB API key](https://www.abuseipdb.com/account/api) (1000 reports/day on the free tier, some accounts get more) and Docker, or a systemd host.

```bash
git clone https://github.com/elhenro/endlessh-auto-report-abuseipdb.git
cd endlessh-auto-report-abuseipdb
./install.sh              # docker; add --systemd for a bare-metal service, --dry-run to try without a key
```

The installer checks what is on port 22, validates your key against AbuseIPDB, writes a private `.env`, starts the tarpit and connects to it once to prove it works. It never edits your sshd config.

Prebuilt multi-arch image (amd64, arm64): `ghcr.io/elhenro/endlessh-auto-report-abuseipdb`

```bash
# without the installer
cp .env.example .env && chmod 600 .env && $EDITOR .env
docker compose pull && docker compose up -d
```

## Before you use port 22

Bots hit port 22, so your real `sshd` has to live elsewhere. **A wrong move here locks you out of your server**, so the installer starts on 2222 whenever 22 is taken. Then:

1. In `sshd_config` set `Port <n>` (1024-65535) and run `sshd -t`.
2. Open `<n>` in your firewall / cloud security group.
3. Reload sshd. From a second terminal confirm `ssh -p <n> you@host` works. Only then close your first session.
4. Re-run with `HOST_PORT=22` (docker: edit `HOST_PORT` in `.env`, then `docker compose up -d`).

## Check that it works

```bash
ssh -p 22 localhost            # hangs forever, never gets a banner: that is the tarpit
docker compose logs -f         # or: journalctl -fu endlessh-reporter
```

Every visitor gets one log line:

```
tarpit: connect 203.0.113.9 -> reported (quota left: 987)
tarpit: connect 203.0.113.9 -> skipped (reported < 900s ago)
tarpit: connect 192.168.65.1 -> skipped (private/reserved address)
```

## Bare metal (systemd)

```bash
sudo ./install.sh --systemd
```

Installs to `/opt/endlessh-reporter` as a sandboxed service: dynamic unprivileged user, only `CAP_NET_BIND_SERVICE` (so it can bind :22 without root), read-only filesystem, token passed as a systemd credential. Config lives in `/etc/endlessh-reporter/env`. Needs bash 4.4+, `endlessh`, `curl` (the installer offers `apt-get`).

## Configuration

Environment variables (`.env` for docker, `/etc/endlessh-reporter/env` for systemd):

| variable | default | meaning |
|---|---|---|
| `API_TOKEN` | | AbuseIPDB key. Required unless `DRY_RUN=1` |
| `API_TOKEN_FILE` | | read the key from this file instead |
| `DRY_RUN` | `0` | `1` logs visitors, sends nothing |
| `ALLOWLIST` | | comma separated plain IPs never reported, e.g. your own (no CIDR) |
| `HOST_PORT` | `22` | docker only: host port to publish |
| `TARPIT_PORT` | `2222` | systemd/bare metal: port endlessh binds |
| `TARPIT_DELAY_MS` | `10000` | endlessh delay between banner lines |
| `TARPIT_MAX_CLIENTS` | `4096` | endlessh client limit |
| `ENDLESSH_ARGS` | | extra endlessh flags, e.g. `-4` |
| `DEDUPE_SECONDS` | `900` | do not re-report an IP within this window |
| `CATEGORIES` | `14,22` | AbuseIPDB categories (`14` port scan, `22` SSH) |
| `REPORTED_FILE` | `reportedIps.txt` | audit log of successful reports, empty disables |
| `CURL_TIMEOUT` | `10` | seconds before an API call is abandoned |
| `EVENTS_DIR` | | write a [JSONL event log](#event-log) here, empty disables (systemd default: `/var/lib/endlessh-reporter/events`) |
| `EVENTS_RETENTION_DAYS` | `30` | delete event files older than this |

## What gets reported

Every TCP connection to the tarpit port, once per 15 minutes per IP. endlessh never sees credentials, so the report says "Connected to SSH tarpit (endlessh), no login attempted" with categories port scan and SSH, not brute force. Internet scanners (Shodan, Censys, research crawlers) get reported too; add your own monitoring IPs to `ALLOWLIST`.

Skipped: private, loopback, link-local, CGNAT and multicast addresses. When AbuseIPDB answers 429 (daily quota) reporting pauses until the quota resets instead of hammering the API.

## Event log

With `EVENTS_DIR` set, every visit is also written to `events-YYYY-MM-DD.jsonl` (UTC days, one JSON object per line), for dashboards or your own `jq`:

```json
{"t":1760000000,"e":"accept","ip":"203.0.113.7","a":"reported","q":2995}
{"t":1760000812,"e":"close","ip":"203.0.113.7","s":812.345,"b":4096}
```

| field | meaning |
|---|---|
| `t` | unix time |
| `e` | `accept` (bot connected) or `close` (bot gave up or was dropped) |
| `a` | what happened to an accept: `reported`, `dedup` (already reported recently), `dry`, `paused` (quota exhausted), `failed`, `rejected` |
| `q` | reports left today, on `reported` accepts |
| `s`, `b` | seconds the bot was held and bytes endlessh sent it, on `close` |

Private, loopback and allowlisted addresses are never written. Files are world readable (`0644`) so a dashboard under another user can read them. systemd writes them by default; for docker add `EVENTS_DIR=/events` and a writable volume for the container user (uid 65534). Files older than `EVENTS_RETENTION_DAYS` are deleted.

## Troubleshooting

| log / symptom | cause |
|---|---|
| `endlessh: ... Address already in use` | something holds the port, usually sshd on 22 |
| `AbuseIPDB rejected the API token (HTTP 401)` | wrong key |
| only `192.168.x` / `172.x` visitors | docker rewrites the source IP (Docker Desktop, rootless docker). Nothing real gets reported. Use a Linux server, or `--systemd` |
| `pausing reports for ...s: daily quota exhausted` | free tier is 1000 reports/day |
| `docker compose pull` says denied / not found | image not published yet: `./install.sh --build` |
| systemd: `cannot read .../api_token` | kernel without tmpfs ACLs; put `API_TOKEN=...` in `/etc/endlessh-reporter/env` (`chmod 600`) |

## Security and privacy

- Docker: runs as `nobody`, read-only rootfs, all capabilities dropped, memory and pid limits. The key is passed to curl via a config descriptor, so it never shows up in `ps` or `/proc/<pid>/cmdline`.
- Only strictly validated IP addresses leave the log parser; nothing from the network reaches a shell.
- Keep `.env` at `chmod 600`. Report vulnerabilities per [SECURITY.md](SECURITY.md).
- IP addresses are personal data under the GDPR. Reporting abusive IPs for network security is commonly covered by legitimate interest, but that call is yours. Docker logs rotate at 10 MB x 3; `reportedIps.txt` grows until you trim it; event files are deleted after `EVENTS_RETENTION_DAYS`.

## Uninstall

```bash
docker compose down -v                                              # docker
sudo systemctl disable --now endlessh-reporter && sudo rm -rf /opt/endlessh-reporter /etc/endlessh-reporter /etc/systemd/system/endlessh-reporter.service   # systemd
```

## Development

`./tests/run.sh` runs the offline test suite (stubbed `curl` and `endlessh`, no network). CI runs shellcheck, hadolint, the tests and a Trivy scan.

## License

MIT
