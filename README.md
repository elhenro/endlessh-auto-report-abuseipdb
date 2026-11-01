# endlessh auto-report to AbuseIPDB

[![ci](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/ci.yml/badge.svg)](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/ci.yml)
[![image](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/docker.yml/badge.svg)](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/docker.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A small SSH tarpit in Rust: one binary, no endlessh, no bash. It keeps bots busy on port 22 and reports their IPs to [AbuseIPDB](https://www.abuseipdb.com/) and/or to any HTTPS webhook, at most once per 15 minutes per IP. The idea comes from [endlessh](https://github.com/skeeto/endlessh).

## Quickstart

You need Docker, or a systemd host with a Rust toolchain (to build the binary once). Reporting is optional: with a free [AbuseIPDB key](https://www.abuseipdb.com/account/api) (1000 reports/day) it reports there, with `WEBHOOK_URL` it also posts to your endpoint, with neither it is a plain tarpit.

```bash
git clone https://github.com/elhenro/endlessh-auto-report-abuseipdb.git
cd endlessh-auto-report-abuseipdb
./install.sh         # --systemd for a bare-metal service, --dry-run to try without a key
```

The installer checks what is on port 22, tests your key, writes a private `.env` and `secrets/api_token`, starts the tarpit and connects to it once. On any port other than 22 it starts with `DRY_RUN=1`, so nothing is reported until you move it there ([why](#what-gets-reported)). It never edits your sshd config.

Image: `ghcr.io/elhenro/endlessh-auto-report-abuseipdb` (amd64, arm64). `:latest` moves with version tags, `:edge` with master. Without the installer: `cp .env.example .env && chmod 600 .env`, edit it, then `docker compose pull && docker compose up -d`. To keep the key out of `docker inspect`, leave `API_TOKEN` empty and add `docker-compose.secrets.yml` to `COMPOSE_FILE` (create `secrets/api_token` with `chmod 644`).

## Upgrading from the bash version

Nothing has to change: an old `.env` or `/etc/endlessh-reporter/env` keeps working, on the same volume with the same `reportedIps.txt`, and going back to the old image works too. Docker: `docker compose pull && docker compose up -d`. Systemd: `git pull && sudo ./install.sh --systemd`.

Old settings, flags, exit codes and `tarpitReporter.sh` all still work. The new binary also reads the existing `reportedIps.txt` at start, so an IP the bash version reported recently is not reported again. What differs, on purpose:

| what | old | now | to get the old behaviour |
|---|---|---|---|
| who gets reported | every TCP connection | only clients that open with `SSH-` ([why](#what-gets-reported)) | `REPORT_ALL_CONNECTIONS=1` |
| clients per source | no limit | `TARPIT_MAX_PER_IP`, 8x per /24, half the slots for private addresses | raise `TARPIT_MAX_PER_IP` |
| IPv6 repeats | per address | per /64 | none |
| systemd: changing `TARPIT_PORT` | `systemctl restart` | re-run the installer (a socket unit holds the port) | none |
| fresh install off port 22 | reports | `DRY_RUN=1` | `DRY_RUN=0 ./install.sh` |
| image internals | shell and tools | none; read the file with `docker run --rm -v <project>_reported:/data debian:stable-slim cat /data/reportedIps.txt` | pin `:1.0.0` |
| `ENDLESSH_ARGS=-f <file>` | endlessh read it | warning, not read: use `TARPIT_*` | none |

The report body is byte for byte v1's, so old and new rows on abuseipdb read the same; the only filters that changed are the two above.

Once, in `.env`, to turn on the two new options: `COMPOSE_FILE=docker-compose.yml:docker-compose.secrets.yml:docker-compose.ipv6.yml`, then `docker compose up -d` (key in a file; IPv6 clients visible to docker's proxy).

## Before you use port 22

Bots hit 22, so your real `sshd` must listen elsewhere. A wrong move here locks you out, so the installer starts on 2222 whenever 22 is taken. Then:

1. In `sshd_config` set `Port <n>` (1024-65535) and run `sshd -t`. On a socket-activated sshd (Ubuntu 22.10 and later) set `ListenStream=<n>` in the `ssh.socket` override instead, then `systemctl daemon-reload`. The installer reads sshd's port from the enabled unit either way.
2. Open `<n>` in your firewall or cloud security group.
3. Reload sshd. From a second terminal check `ssh -p <n> you@host`. Only then close your first session.
4. Docker: `HOST_PORT=22` and `DRY_RUN=0` in `.env`, then `./install.sh`. Systemd: the same in `/etc/endlessh-reporter/env`, then `sudo ./install.sh --systemd`.

Use the installer for step 4, not `docker compose up` or `systemctl`: it refuses while anything else holds the port or sshd still uses it. A socket unit left enabled on a port sshd uses looks harmless, but at the next boot it takes the port first and sshd does not start.

## Check that it works

```bash
ssh -p 22 localhost      # hangs forever, no banner: that is the tarpit
docker compose logs -f   # or: journalctl -fu endlessh-reporter
```

Every visitor logs one line. Reports always get their own line; connections skipped for the same reason are logged once and then at most once a minute with a count, so a port scan cannot fill the log.

```
tarpit: connect 203.0.113.9 -> reported (quota left: 987)
tarpit: connect 192.168.65.1 -> skipped (private/reserved address)
tarpit: 412 connections skipped, last 192.168.65.1 (private/reserved address)
```

## Configuration

`.env` for docker, `/etc/endlessh-reporter/env` for systemd.

| variable | default | meaning |
|---|---|---|
| `API_TOKEN` | | AbuseIPDB key; prefer `API_TOKEN_FILE` (or the systemd credential), since `docker inspect` shows the environment |
| `WEBHOOK_URL` | | also `POST` `{"event":"accept","t":..,"ip":"..","port":..}` per new visitor; `https://` only (`http://` for localhost) |
| `WEBHOOK_AUTH` | | full `Authorization` header value, e.g. `Bearer abc` (`WEBHOOK_AUTH_FILE` too) |
| `DRY_RUN` | `0` | `1` logs visitors, sends nothing |
| `REPORT_ALL_CONNECTIONS` | `0` | `1` reports every connection, not only `SSH-` clients |
| `ALLOWLIST` | | comma separated IPs or CIDR ranges never reported, e.g. your own |
| `HOST_PORT` / `TARPIT_PORT` | `22` / `2222` | docker host port / systemd port the tarpit binds. Other than 22 logs a warning, and the installer uses `DRY_RUN=1` |
| `ENDLESSH_ARGS` | | endlessh flags for old configs: `-4`, `-6`, `-d`, `-l`, `-m`, `-p`; they win over these variables |
| `TARPIT_DELAY_MS` | `10000` | delay between banner lines |
| `TARPIT_MAX_CLIENTS` | `4096` | stuck clients at once; one fd each, so past 8000 raise `LimitNOFILE` / `ulimits` too |
| `TARPIT_MAX_PER_IP` | `32` | clients per IP (IPv6: /64), 8x per /24 (IPv6: /48); private addresses share at most half of `TARPIT_MAX_CLIENTS` |
| `TARPIT_MAX_LINE` | `32` | max bytes per banner line (3-255) |
| `DEDUPE_SECONDS` | `900` | no repeat report within this window (AbuseIPDB refuses repeats under 15 minutes anyway) |
| `CATEGORIES` | `18,22` | AbuseIPDB categories: `18` brute force, `22` SSH; same as v1 |
| `HTTP_TIMEOUT` | `10` | seconds per API call (1-120); old name `CURL_TIMEOUT` works |
| `REPORTED_FILE` | `/data/reportedIps.txt`, systemd `/var/lib/endlessh-reporter/reportedIps.txt` | what AbuseIPDB accepted, `<time> <ip>` per line; empty turns it off; a symlink is refused |
| `EVENTS_DIR` | systemd `/var/lib/endlessh-reporter/events` | write the [event log](#event-log) here; empty turns it off |
| `EVENTS_RETENTION_DAYS` | `30` | delete older event files |
| `EVENTS_MAX_DAILY_MB` | `100` | stop writing events for the UTC day past this size |

## What gets reported

A connection is reported (once per 15 minutes per IP, per /64 for IPv6) if its first bytes are `SSH-`, about 10 seconds after it connects. The report is the one the bash version has always sent: categories `18,22` and the comment `SSH login attempts (endlessh): ACCEPT host=<ip> port=<port>`. The tarpit never reads past those first bytes, so the comment is what endlessh's own log said, not a claim about a password being tried.

**Not reported, because it could be anybody:** a connection that opens with anything else (HTTP, TLS), says nothing within 10 seconds, or closes silently (`nc -z`, port checkers). Otherwise a web page or link preview bot could be pointed at any host and whoever runs it would get reported. The tarpit still holds them, it just stays quiet. It peeks at the first 4 bytes and compares them with `SSH-`; they stay unread in the kernel. OpenSSH, libssh, libssh2 and paramiko speak first; nmap's `ssh-hostkey` does not and is skipped.

Port 22 still matters: browsers refuse it, and on other ports a cross-site `fetch` sends `OPTIONS` before it could send `SSH-`. DNS rebinding is the exception, which is why the installer uses `DRY_RUN=1` on other ports and the binary warns if you report there.

Private, loopback, link-local, CGNAT, multicast and reserved addresses are skipped. A 429 for the daily quota pauses reporting until it resets; a 429 for a repeat settles that IP only; a rejected key pauses for an hour, other API failures for 30 seconds. API calls run on their own thread, so a slow API never slows the tarpit.

## Event log

With `EVENTS_DIR` set, every visit also goes to `events-YYYY-MM-DD.jsonl` (UTC days, one JSON object per line, `0644`):

```json
{"t":1760000000,"e":"accept","ip":"203.0.113.7","a":"reported","q":2995}
{"t":1760000812,"e":"close","ip":"203.0.113.7","s":812.345,"b":4096}
```

`e` is `accept` or `close`; `a` is `reported`, `delivered`, `dedup`, `dry`, `paused`, `failed`, `rejected` or `notssh`; `q` is the quota left; `s` and `b` are seconds kept and bytes sent. Private, loopback and allowlisted addresses are never written, and a prepared file name (symlink, hard link, fifo, other owner) is refused. systemd writes them under `/var/lib/private` (`0700`), so a dashboard needs root or a bind mount; for docker set `EVENTS_DIR=/data/events`. Past `EVENTS_MAX_DAILY_MB` events stop for the day, so a loop cannot fill the disk.

## Troubleshooting

| log / symptom | cause |
|---|---|
| `Address already in use` | something holds the port, usually sshd on 22 |
| `rejected the API token (HTTP 401)` | wrong key |
| only `192.168.x` / `172.x` / `fd..` visitors | docker rewrites the source IP (Docker Desktop, rootless, IPv6 before Docker 27): nothing real is reported. Use a Linux server with current Docker, or `--systemd` |
| `at TARPIT_MAX_CLIENTS ... backlog` | every slot is taken; normal under a scan |
| `could not start the tarpit` with ipv6 | kernel has IPv6 off: drop `docker-compose.ipv6.yml` from `COMPOSE_FILE` |
| `daily quota exhausted` | free tier is 1000 reports/day |
| `bind source path does not exist: .../secrets/api_token` | create it: `mkdir -m 700 secrets && touch secrets/api_token && chmod 644 secrets/api_token` |
| `TARPIT_PORT=N is not in effect` | systemd holds another port: re-run the installer |
| `out of file descriptors` | `TARPIT_MAX_CLIENTS` above the limit: raise `LimitNOFILE` / `ulimits` |
| exit status 78 | a setting is invalid, the log names it; systemd stops restarting once the start limit fails the socket. Fix it and re-run the installer |

## Security

Static binary, distroless, `nobody`, read-only rootfs, no capabilities, about 4 MiB of RAM; systemd runs it as a dynamic user with a syscall filter, and a socket unit owns the port. It never reads from a client and never parses what a client sends. The key stays in memory and one request header. Report vulnerabilities per [SECURITY.md](SECURITY.md). Keep `.env` at `chmod 600`. IPs are personal data under the GDPR: reporting abusive IPs is usually legitimate interest, but that call is yours.

## Uninstall

```bash
docker compose down -v && rm -rf secrets .env                       # docker
sudo systemctl disable --now endlessh-reporter.socket endlessh-reporter && sudo rm -rf /opt/endlessh-reporter /etc/endlessh-reporter /etc/systemd/system/endlessh-reporter.service /etc/systemd/system/endlessh-reporter.socket /var/lib/private/endlessh-reporter /var/lib/endlessh-reporter   # systemd
```

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Tests run offline against local mock APIs. CI adds shellcheck, hadolint, a Trivy scan and a daily `cargo audit`. Code map: `accept.rs`/`slots.rs` (connections, limits), `tarpit.rs` (banner), `hello.rs` (is it SSH), `worker.rs` (filter, dedupe, sinks), `sinks.rs`, `abuseipdb.rs`, `webhook.rs`, `events.rs`, `ip.rs`, `config.rs`. Another API is one `impl Sink` plus a line in `main.rs`.

## License

MIT
