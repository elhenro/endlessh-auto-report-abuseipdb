# endlessh auto-report to AbuseIPDB

[![ci](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/ci.yml/badge.svg)](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/ci.yml)
[![image](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/docker.yml/badge.svg)](https://github.com/elhenro/endlessh-auto-report-abuseipdb/actions/workflows/docker.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A small SSH tarpit in Rust (one small binary, no endlessh, no bash) that stalls bots on port 22 and optionally reports their IPs to [AbuseIPDB](https://www.abuseipdb.com/) and/or any HTTPS webhook. Each IP is reported at most once per 15 minutes. The tarpit idea and banner trick come from [endlessh](https://github.com/skeeto/endlessh).

## Quickstart

You need Docker, or a systemd host plus a Rust toolchain to build once. Reporting is optional: with a free [AbuseIPDB API key](https://www.abuseipdb.com/account/api) (1000 reports/day on the free tier) it reports to AbuseIPDB, with `WEBHOOK_URL` it also posts to your endpoint, with neither it is a plain tarpit.

```bash
git clone https://github.com/elhenro/endlessh-auto-report-abuseipdb.git
cd endlessh-auto-report-abuseipdb
./install.sh              # docker; add --systemd for a bare-metal service, --dry-run to try without a key
```

The installer checks what is on port 22, validates your key against AbuseIPDB, writes a private `.env` and `secrets/api_token`, starts the tarpit and connects to it once to prove it works. It never edits your sshd config.

Prebuilt multi-arch image (amd64, arm64): `ghcr.io/elhenro/endlessh-auto-report-abuseipdb`

```bash
# without the installer
cp .env.example .env && chmod 600 .env && $EDITOR .env
mkdir -m 700 secrets && touch secrets/api_token && chmod 644 secrets/api_token
$EDITOR secrets/api_token     # your AbuseIPDB key; leave empty for no AbuseIPDB reporting
docker compose pull && docker compose up -d
```

## Before you use port 22

Bots hit port 22, so your real `sshd` has to live elsewhere. **A wrong move here locks you out of your server**, so the installer starts on 2222 whenever 22 is taken. Then:

1. In `sshd_config` set `Port <n>` (1024-65535) and run `sshd -t`.
2. Open `<n>` in your firewall / cloud security group.
3. Reload sshd. From a second terminal confirm `ssh -p <n> you@host` works. Only then close your first session.
4. Docker: set `HOST_PORT=22` in `.env`, then `docker compose up -d`. Systemd: set `TARPIT_PORT=22` in `/etc/endlessh-reporter/env`, then `sudo ./install.sh --systemd`.

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

Installs the binary to `/opt/endlessh-reporter` as a sandboxed service: dynamic unprivileged user, no capabilities at all (a `.socket` unit owns the port, so systemd binds :22 and hands the service the listener), read-only filesystem, syscall filter, token passed as a systemd credential. Config lives in `/etc/endlessh-reporter/env`; the port is set from `TARPIT_PORT` at install time, so re-run the installer to change it. Build first as your own user (`cargo build --release --locked`), or let the installer build it if `cargo` is on root's PATH. Upgrading from the old bash version removes its leftovers.

## Configuration

Environment variables (`.env` for docker, `/etc/endlessh-reporter/env` for systemd):

| variable | default | meaning |
|---|---|---|
| `API_TOKEN` | | AbuseIPDB key. Without one (and without `WEBHOOK_URL`) it runs as a plain tarpit. Prefer the file below: `docker inspect` shows the environment |
| `API_TOKEN_FILE` | | read the key from this file instead (compose: `secrets/api_token`; systemd: `$CREDENTIALS_DIRECTORY/api_token` is picked up automatically) |
| `WEBHOOK_URL` | | also `POST` `{"event":"accept","t":..,"ip":"..","port":..}` here for every fresh visitor. `https://` only (`http://` for localhost) |
| `WEBHOOK_AUTH` | | full `Authorization` header value, e.g. `Bearer abc` (also `WEBHOOK_AUTH_FILE` / credential `webhook_auth`) |
| `DRY_RUN` | `0` | `1` logs visitors, sends nothing anywhere |
| `ALLOWLIST` | | comma separated IPs or CIDR ranges never reported, e.g. your own |
| `HOST_PORT` | `22` | docker only: host port to publish. A value other than 22 logs a [warning](#what-gets-reported) when reporting |
| `TARPIT_PORT` | `2222` | systemd/bare metal: port the tarpit binds |
| `TARPIT_DELAY_MS` | `10000` | delay between banner lines |
| `TARPIT_MAX_CLIENTS` | `4096` | simultaneous stuck clients; beyond that new connections wait in the kernel backlog. Each takes one fd: past 8000 also raise `LimitNOFILE` (systemd) or `ulimits` (compose) |
| `TARPIT_MAX_PER_IP` | `32` | simultaneous clients per public IP (IPv6: per /64), so one host cannot take every slot. Connections over it are closed at once and only counted in a log line once a minute. Private addresses are exempt, since behind a docker proxy every visitor looks the same |
| `TARPIT_MAX_LINE` | `32` | max bytes per random banner line (3-255) |
| `DEDUPE_SECONDS` | `900` | do not re-report an IP (IPv6: a /64) within this window. AbuseIPDB itself refuses repeats within 15 minutes, so lower values only add useless API calls |
| `CATEGORIES` | `14,22` | AbuseIPDB categories (`14` port scan, `22` SSH) |
| `ABUSEIPDB_URL` | `https://api.abuseipdb.com/api/v2` | API base, for tests or a proxy |
| `HTTP_TIMEOUT` | `10` | seconds before an API call is abandoned |
| `EVENTS_DIR` | | write a [JSONL event log](#event-log) here, empty disables (systemd default: `/var/lib/endlessh-reporter/events`) |
| `EVENTS_RETENTION_DAYS` | `30` | delete event files older than this |
| `EVENTS_MAX_DAILY_MB` | `100` | stop writing events for the rest of the UTC day past this size, so disk use stays below this times the retention |

## What gets reported

Every TCP connection to the tarpit port, once per 15 minutes per IP, or per /64 for IPv6, since rotating inside a /64 is free. Someone holding a larger IPv6 block can still spend your daily quota. The tarpit never sees credentials, so the report says "Connected to SSH tarpit, no login attempted" with categories port scan and SSH, not brute force. Internet scanners (Shodan, Censys, research crawlers) get reported too; add your own monitoring IPs to `ALLOWLIST`.

**A bare connect is enough for a report, and it does not have to be the owner of that IP who wanted it.** Browsers refuse to connect to port 22, but not to 2222: any web page can make its visitors' browsers connect to a tarpit there, and those visitors get reported. Link preview bots can be steered the same way. So the binary and the installer warn when reporting on any other port; use `DRY_RUN=1` until the tarpit is on 22. Reading the client's first bytes could tell these cases apart, but the tarpit never reads, by design.

Skipped: private, loopback, link-local, CGNAT, multicast, documentation and reserved addresses. When AbuseIPDB answers 429 for the daily quota, reporting pauses until the quota resets instead of hammering the API. A 429 for a repeat (same IP within 15 minutes, for example after a restart or from a second host on your key) settles that IP and pauses nothing. Any other API failure, including a redirect, pauses reporting for 30 seconds. API calls run on their own thread, so a slow or dead API never slows the tarpit. Visits dropped while the worker queue is full (4096) are counted in a log warning.

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
| `a` | what happened to an accept: `reported` (AbuseIPDB took it), `delivered` (webhook only), `dedup` (already reported recently), `dry` (no sink), `paused` (quota exhausted), `failed`, `rejected` |
| `q` | reports left today, on `reported` accepts |
| `s`, `b` | seconds the bot was held and bytes sent to it, on `close` |

Private, loopback and allowlisted addresses are never written. Files are `0644` and a new `EVENTS_DIR` is `0755`; an existing one keeps its mode, and is refused (events off, with a warning) if group or others can write to it. systemd writes events by default, but under `/var/lib/private` (`0700`, from `DynamicUser`), so a dashboard must run as root or read them through a bind mount. For docker set `EVENTS_DIR=/data/events` (the `/data` volume is writable for uid 65534). Past `EVENTS_MAX_DAILY_MB` new events are dropped until the next UTC day, with one log warning, so a host looping connects can blank the log for a day but cannot fill the disk. The first configured sink (AbuseIPDB, else the webhook) decides the `a` value. Files older than `EVENTS_RETENTION_DAYS` are deleted. Lines are written unbuffered but not fsynced: a process crash loses nothing, a power loss can lose the last few seconds.

## Troubleshooting

| log / symptom | cause |
|---|---|
| `cannot listen on port N: Address already in use` | something holds the port, usually sshd on 22 |
| `AbuseIPDB rejected the API token (HTTP 401)` | wrong key |
| only `192.168.x` / `172.x` visitors | docker rewrites the source IP (Docker Desktop, rootless docker). Nothing real gets reported. Use a Linux server, or `--systemd` |
| `pausing AbuseIPDB reports for ...s: daily quota exhausted` | free tier is 1000 reports/day |
| `bind source path does not exist: .../secrets/api_token` | `mkdir -m 700 secrets && touch secrets/api_token && chmod 644 secrets/api_token` |
| `cannot read /run/secrets/api_token: Permission denied` | the file must be `0644`, its directory keeps it private: `chmod 644 secrets/api_token` |
| `out of file descriptors, holding at most N clients` | `TARPIT_MAX_CLIENTS` above the fd limit: raise `LimitNOFILE` / `ulimits` or lower it, then restart |
| `docker compose pull` says denied / not found | image not published yet: `./install.sh --build` |
| systemd: `cannot read .../api_token` | kernel without tmpfs ACLs; put `API_TOKEN=...` in `/etc/endlessh-reporter/env` and `chmod 600` it |
| exit status 78 | a setting is invalid; the log line says which. systemd does not restart on 78, but each visitor triggers a start through the socket until the start limit fails the socket too and the port closes. Fix the setting, then re-run the installer |

## Security and privacy

- Docker: fully static binary on a distroless image, so no libc, shell or package manager. Runs as `nobody`, read-only rootfs, all capabilities dropped, memory and pid limits (about 4 MiB of RAM in practice).
- The tarpit never reads from a client, so nothing a client sends is ever parsed; the only thing taken from a connection is its address. The only network input parsed at all is the HTTPS response from AbuseIPDB or your webhook: status and headers, plus at most 200 bytes of an AbuseIPDB error body, logged with control characters blanked. The API key lives in memory and in one request header, never in argv or in a child process environment. API calls refuse redirects and plain http (except localhost).
- No `unsafe` besides one `from_raw_fd` for systemd socket activation. A panic in any thread aborts the process so systemd restarts it.
- Keep `.env` at `chmod 600` and `secrets/` at `chmod 700`. Report vulnerabilities per [SECURITY.md](SECURITY.md).
- IP addresses are personal data under the GDPR. Reporting abusive IPs for network security is commonly covered by legitimate interest, but that call is yours. Docker logs rotate at 10 MB x 3 and event files are deleted after `EVENTS_RETENTION_DAYS`. Nothing else is stored; the old `reportedIps.txt` audit file is gone (the event log covers it, with retention).

## Uninstall

```bash
docker compose down -v && rm -rf secrets .env                       # docker
sudo systemctl disable --now endlessh-reporter.socket endlessh-reporter && sudo rm -rf /opt/endlessh-reporter /etc/endlessh-reporter /etc/systemd/system/endlessh-reporter.service /etc/systemd/system/endlessh-reporter.socket   # systemd
```

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

The tests run offline: unit tests plus end-to-end tests against local mock APIs and a loopback tarpit. CI additionally runs shellcheck on the installers, hadolint, a Trivy scan of the image and a daily `cargo audit`.

Code map: `accept.rs` and `slots.rs` (taking connections, client limits), `tarpit.rs` (banner loop), `worker.rs` (filter, dedupe, fan out to sinks, event log), `sinks.rs` (the `Sink` trait), `abuseipdb.rs` and `webhook.rs` (the two sinks), `events.rs`, `ip.rs`, `config.rs`. Another API is one `impl Sink` plus a line in `main.rs`.

## License

MIT
