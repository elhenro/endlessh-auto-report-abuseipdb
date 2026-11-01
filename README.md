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

The installer checks what is on port 22, validates your key against AbuseIPDB, writes a private `.env` and `secrets/api_token` (and turns those on through `COMPOSE_FILE`), starts the tarpit and connects to it once to prove it works. On any port other than 22 it starts with `DRY_RUN=1`, so nothing is reported until you move it there ([why](#what-gets-reported)). It never edits your sshd config.

Prebuilt multi-arch image (amd64, arm64): `ghcr.io/elhenro/endlessh-auto-report-abuseipdb`. `:latest` moves with version tags, `:edge` with master. To check where an image came from: `gh attestation verify oci://ghcr.io/elhenro/endlessh-auto-report-abuseipdb:latest --owner elhenro`.

```bash
# without the installer
cp .env.example .env && chmod 600 .env && $EDITOR .env
docker compose pull && docker compose up -d
```

To keep the key out of `docker inspect`, leave `API_TOKEN` empty, add `docker-compose.secrets.yml` to `COMPOSE_FILE` in `.env`, and:

```bash
mkdir -m 700 secrets && touch secrets/api_token && chmod 644 secrets/api_token
$EDITOR secrets/api_token
```

## Upgrading from the bash version

Nothing has to change. A `.env` or `/etc/endlessh-reporter/env` from the old version keeps working as it is, on the same volume with the same audit file, and going back to the old image works too. Docker: `docker compose pull && docker compose up -d`, with or without a `git pull` first. Systemd: `git pull && sudo ./install.sh --systemd`, which takes the binary from the latest release if the host has no Rust toolchain.

Every old setting still works: `API_TOKEN`, `API_TOKEN_FILE`, `DRY_RUN`, `ALLOWLIST`, `HOST_PORT`, `TARPIT_*`, `DEDUPE_SECONDS`, `CATEGORIES`, `REPORTED_FILE`, `CURL_TIMEOUT` and `ENDLESSH_ARGS`. So do the installer's flags, the log lines above, exit codes, and `tarpitReporter.sh` as an entry point.

What behaves differently, on purpose:

| what | old | now | to get the old behaviour |
|---|---|---|---|
| who gets reported | every TCP connection | only clients that open with `SSH-` ([why](#what-gets-reported)), about 10 seconds after they connect | `REPORT_ALL_CONNECTIONS=1` |
| default `CATEGORIES`, report comment | `18,22`, "SSH login attempts" | `14,22`, "Connected to SSH tarpit, no login attempted": the tarpit never sees a login | `CATEGORIES=18,22` (the comment stays) |
| clients per source | no limit | `TARPIT_MAX_PER_IP`, 8 times that per /24 or /48, half of all slots for private addresses | raise `TARPIT_MAX_PER_IP` |
| repeat reports on IPv6 | per address | per /64 | none |
| systemd: changing `TARPIT_PORT` | `systemctl restart` | re-run the installer, because a socket unit holds the port. The service logs a warning if they differ. The installer refuses a port an enabled sshd will take | none |
| fresh install on a port other than 22 | reports | `DRY_RUN=1` | `DRY_RUN=0 ./install.sh` |
| inside the image | a shell and tools | none: `docker exec ... sh` and `cat` are gone. Read the audit file with `docker run --rm -v <project>_reported:/data debian:stable-slim cat /data/reportedIps.txt` | pin the old image, `:1.0.0` |
| `ENDLESSH_ARGS=-f <file>` | endlessh read that config file | a warning, the file is not read: use the `TARPIT_*` settings | none |

Optional, and only needed once: after a `git pull`, three things are off for an old `.env` until you add them, because each needs a change on your side.

- **Key in a file:** move it out of `API_TOKEN` as in the [quickstart](#quickstart), so `docker inspect` stops showing it.
- **IPv6 clients:** without `docker-compose.ipv6.yml` docker's proxy hides them behind one address. They then share the slots for private addresses and do not get reported.
- Both go in one line of `.env`: `COMPOSE_FILE=docker-compose.yml:docker-compose.secrets.yml:docker-compose.ipv6.yml`, then `docker compose up -d`.

Without a `git pull` the old compose file also keeps `init: true`, and that init process stays readable through `/proc` for a host service running as `nobody`, as it always was.

## Before you use port 22

Bots hit port 22, so your real `sshd` has to live elsewhere. **A wrong move here locks you out of your server**, so the installer starts on 2222 whenever 22 is taken. Then:

1. In `sshd_config` set `Port <n>` (1024-65535) and run `sshd -t`.
2. Open `<n>` in your firewall / cloud security group.
3. Reload sshd. From a second terminal confirm `ssh -p <n> you@host` works. Only then close your first session.
4. Docker: set `HOST_PORT=22` and `DRY_RUN=0` in `.env`, then `./install.sh`. Systemd: set `TARPIT_PORT=22` and `DRY_RUN=0` in `/etc/endlessh-reporter/env`, then `sudo ./install.sh --systemd`.

Use the installer for step 4, not `docker compose up` or `systemctl` by hand: it refuses while anything else holds the port or sshd is still configured for it. A socket unit left enabled on a port sshd uses looks harmless, but at the next boot it gets the port first and sshd does not start.

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

Installs the binary to `/opt/endlessh-reporter` as a sandboxed service: dynamic unprivileged user, no capabilities at all (a `.socket` unit owns the port, so systemd binds :22 and hands the service the listener), read-only filesystem, syscall filter, token passed as a systemd credential. Config lives in `/etc/endlessh-reporter/env`; the port is set from `TARPIT_PORT` at install time, so re-run the installer to change it. The installer uses a binary you built (`cargo build --release --locked`, as your own user), builds it if `cargo` is on root's PATH, and otherwise offers the one from the latest release. A binary older than the source is ignored, so after `git pull` build again before re-running. Upgrading from the old bash version removes its program files and keeps its `reportedIps.txt`.

## Configuration

Environment variables (`.env` for docker, `/etc/endlessh-reporter/env` for systemd):

| variable | default | meaning |
|---|---|---|
| `API_TOKEN` | | AbuseIPDB key. Without one (and without `WEBHOOK_URL`) it runs as a plain tarpit. Prefer the file below: `docker inspect` shows the environment |
| `API_TOKEN_FILE` | | read the key from this file instead (compose: `docker-compose.secrets.yml` sets it; systemd: `$CREDENTIALS_DIRECTORY/api_token` is picked up automatically) |
| `WEBHOOK_URL` | | also `POST` `{"event":"accept","t":..,"ip":"..","port":..}` here for every fresh visitor. `https://` only (`http://` for localhost) |
| `WEBHOOK_AUTH` | | full `Authorization` header value, e.g. `Bearer abc` (also `WEBHOOK_AUTH_FILE` / credential `webhook_auth`) |
| `DRY_RUN` | `0` | `1` logs visitors, sends nothing anywhere |
| `REPORT_ALL_CONNECTIONS` | `0` | `1` reports every connection as the bash version did, not only clients that open with `SSH-`. Anything that can be made to connect here then gets whoever runs it [reported](#what-gets-reported) |
| `ALLOWLIST` | | comma separated IPs or CIDR ranges never reported, e.g. your own |
| `HOST_PORT` | `22` | docker only: host port to publish. A value other than 22 logs a [warning](#what-gets-reported) when reporting, and the installer starts it with `DRY_RUN=1` |
| `TARPIT_PORT` | `2222` | systemd/bare metal: port the tarpit binds |
| `ENDLESSH_ARGS` | | endlessh's flags, for old configs: `-4`, `-6`, `-d`, `-l`, `-m`, `-p`. They win over the variables here. `-v` and `-s` do nothing, `-f` is not read (with a warning). Under systemd `-4`, `-6` and `-p` take effect when the installer runs |
| `TARPIT_DELAY_MS` | `10000` | delay between banner lines |
| `TARPIT_MAX_CLIENTS` | `4096` | simultaneous stuck clients; beyond that new connections wait in the kernel backlog and get no report, with a warning in the log once a minute. Each takes one fd: past 8000 also raise `LimitNOFILE` (systemd) or `ulimits` (compose) |
| `TARPIT_MAX_PER_IP` | `32` | simultaneous clients per public IP (IPv6: per /64), and 8 times that per /24 (IPv6: per /48), so one host or one routed block cannot take every slot. Connections over it are closed at once and only counted in a log line once a minute. Private addresses have no limit per address, since behind a docker proxy every visitor looks the same, but together get at most half of `TARPIT_MAX_CLIENTS` |
| `TARPIT_MAX_LINE` | `32` | max bytes per random banner line (3-255) |
| `DEDUPE_SECONDS` | `900` | do not re-report an IP (IPv6: a /64) within this window. AbuseIPDB itself refuses repeats within 15 minutes, so lower values only add useless API calls |
| `CATEGORIES` | `14,22` | AbuseIPDB categories (`14` port scan, `22` SSH) |
| `ABUSEIPDB_URL` | `https://api.abuseipdb.com/api/v2` | API base, for tests or a proxy |
| `HTTP_TIMEOUT` | `10` | seconds before an API call is abandoned, 1-120. The old name `CURL_TIMEOUT` still works, with anything above 120 (or 0, no limit) taken as 120 |
| `REPORTED_FILE` | image: `/data/reportedIps.txt`; systemd: `/var/lib/endlessh-reporter/reportedIps.txt` | audit file of what AbuseIPDB accepted, one `<time> <ip>` per line. Empty disables. Grows until you trim it, and a symlink is refused |
| `EVENTS_DIR` | | write a [JSONL event log](#event-log) here, empty disables (systemd default: `/var/lib/endlessh-reporter/events`) |
| `EVENTS_RETENTION_DAYS` | `30` | delete event files older than this |
| `EVENTS_MAX_DAILY_MB` | `100` | stop writing events for the rest of the UTC day past this size, so disk use stays below this times the retention |

## What gets reported

A connection gets reported, once per 15 minutes per IP (per /64 for IPv6, since rotating inside a /64 is free), if its first bytes are `SSH-`. The report goes out on the tarpit's first tick for that client, about 10 seconds after the connect. The tarpit never sees credentials, so the report says "Connected to SSH tarpit, no login attempted" with categories port scan and SSH, not brute force. Someone holding a larger IPv6 block can still spend your daily quota. Internet scanners that speak SSH get reported too; add your own monitoring IPs to `ALLOWLIST`.

**Not reported, because it could be anybody:** a connection that opens with anything else (HTTP, TLS), says nothing within 10 seconds, or closes without a word (`nc -z`, port checkers). This is what stops the abuse a plain "report every connect" allows: a web page or a link preview bot can be pointed at any host, and it is whoever runs it that would get reported. The tarpit still holds all of them, it only keeps quiet about them. It peeks at the first 4 bytes and compares them with `SSH-`; they stay unread in the kernel and nothing parses them. OpenSSH, libssh, libssh2 and paramiko all speak first. A client that waits for the server to speak first, like nmap's `ssh-hostkey` script, does not get reported.

**Port 22 still matters.** Browsers refuse to connect to 22. On other ports a page cannot normally make a browser send `SSH-` first: a cross-site `fetch` with the method `SSH-2.0` sends `OPTIONS` before it. Through DNS rebinding that request can be same-origin, and then it does start with `SSH-2.0`. So the installer starts with `DRY_RUN=1` on any other port, and the binary warns if you report there anyway.

Skipped: private, loopback, link-local, CGNAT, multicast, documentation and reserved addresses. When AbuseIPDB answers 429 for the daily quota, reporting pauses until the quota resets instead of hammering the API. A 429 for a repeat (same IP within 15 minutes, for example after a restart or from a second host on your key) settles that IP and pauses nothing. A rejected key (401, 403) pauses AbuseIPDB reporting for an hour and any other API failure, including a redirect, for 30 seconds. Each sink keeps its own memory: one that is paused or down neither holds back the other nor makes it repeat itself, and it gets a failed visit again on that visitor's next connect. API calls run on their own thread, so a slow or dead API never slows the tarpit. Visits dropped while the worker queue is full (4096) are counted in a log warning.

## Event log

With `EVENTS_DIR` set, every visit is also written to `events-YYYY-MM-DD.jsonl` (UTC days, one JSON object per line), for dashboards or your own `jq`:

```json
{"t":1760000000,"e":"accept","ip":"203.0.113.7","a":"reported","q":2995}
{"t":1760000812,"e":"close","ip":"203.0.113.7","s":812.345,"b":4096}
```

| field | meaning |
|---|---|
| `t` | unix time of the connect |
| `e` | `accept` (bot connected) or `close` (bot gave up or was dropped) |
| `a` | what happened to an accept: `reported` (AbuseIPDB took it), `delivered` (webhook only), `dedup` (already reported recently), `dry` (no sink), `paused` (quota exhausted), `failed`, `rejected`, `notssh` (never said `SSH-`) |
| `q` | reports left today, on `reported` accepts |
| `s`, `b` | seconds the bot was held and bytes sent to it, on `close` |

Private, loopback and allowlisted addresses are never written. Files are `0644` and a new `EVENTS_DIR` is `0755`; an existing one keeps its mode, and is refused (events off, with a warning) if group or others can write to it. A file name someone prepared in there (symlink, hard link, fifo, or a file of another owner) is refused too, for that day. systemd writes events by default, but under `/var/lib/private` (`0700`, from `DynamicUser`), so a dashboard must run as root or read them through a bind mount. For docker set `EVENTS_DIR=/data/events`. Past `EVENTS_MAX_DAILY_MB` new events are dropped until the next UTC day, with one log warning, so a host looping connects can blank the log for a day but cannot fill the disk. The first configured sink (AbuseIPDB, else the webhook) decides the `a` value. Files older than `EVENTS_RETENTION_DAYS` are deleted. Lines are written unbuffered but not fsynced: a process crash loses nothing, a power loss can lose the last few seconds.

## Troubleshooting

| log / symptom | cause |
|---|---|
| `cannot listen on port N: Address already in use` | something holds the port, usually sshd on 22 |
| `AbuseIPDB rejected the API token (HTTP 401)` | wrong key |
| only `192.168.x` / `172.x` / `fd..` visitors | docker rewrites the source IP (Docker Desktop, rootless docker, IPv6 before Docker 27). Nothing real gets reported and those visitors share half the slots. Use a Linux server with a current Docker, or `--systemd` |
| `at TARPIT_MAX_CLIENTS, new visitors wait in the backlog` | every slot is taken. Normal under a big scan; if `over TARPIT_MAX_PER_IP` lines come with it, look at who holds them |
| `could not start the tarpit` with an ipv6 error | the kernel has IPv6 turned off: take `docker-compose.ipv6.yml` out of `COMPOSE_FILE` in `.env` |
| `pausing reports for ...s: daily quota exhausted` | free tier is 1000 reports/day |
| `bind source path does not exist: .../secrets/api_token` | `COMPOSE_FILE` in `.env` lists `docker-compose.secrets.yml`: `mkdir -m 700 secrets && touch secrets/api_token && chmod 644 secrets/api_token` |
| `TARPIT_PORT=N is not in effect` | systemd holds another port: re-run the installer to move it |
| `cannot read /run/secrets/api_token: Permission denied` | the file must be `0644`, its directory keeps it private: `chmod 644 secrets/api_token` |
| `out of file descriptors, holding at most N clients` | `TARPIT_MAX_CLIENTS` above the fd limit: raise `LimitNOFILE` / `ulimits` or lower it, then restart |
| `docker compose pull` says denied / not found | image not published yet: `./install.sh --build` |
| systemd: `cannot read .../api_token` | kernel without tmpfs ACLs; put `API_TOKEN=...` in `/etc/endlessh-reporter/env` and `chmod 600` it |
| exit status 78 | a setting is invalid; the log line says which. systemd does not restart on 78, but each visitor triggers a start through the socket until the start limit fails the socket too and the port closes. Fix the setting, then re-run the installer |

## Security and privacy

- Docker: fully static binary on a distroless image, so no libc, shell or package manager. Runs as `nobody`, read-only rootfs, all capabilities dropped, memory and pid limits (about 4 MiB of RAM in practice). It marks itself non-dumpable, so a host service running as `nobody` cannot read its memory or the container's files through `/proc`. That service could still send it a signal, and docker would restart it.
- The tarpit never reads from a client, so nothing a client sends is ever parsed; what it takes from a connection is its address and a peek at the first 4 bytes, which stay unread in the kernel and are only compared with `SSH-`. The only network input parsed at all is the HTTPS response from AbuseIPDB or your webhook: status and headers, plus at most 200 bytes of an AbuseIPDB error body, logged with control characters blanked. The API key lives in memory and in one request header, never in argv or in a child process environment. API calls refuse redirects and plain http (except localhost).
- `unsafe` only for `from_raw_fd` on the systemd socket, `prctl` and a signal handler that calls `_exit`. A panic in any thread aborts the process so systemd restarts it.
- Keep `.env` at `chmod 600` and `secrets/` at `chmod 700`. Report vulnerabilities per [SECURITY.md](SECURITY.md).
- IP addresses are personal data under the GDPR. Reporting abusive IPs for network security is commonly covered by legitimate interest, but that call is yours. Docker logs rotate at 10 MB x 3, and event files are deleted after `EVENTS_RETENTION_DAYS` while the tarpit runs. Under systemd the journal keeps visitor IPs for as long as journald is set to keep anything. `reportedIps.txt` grows until you trim it, or set `REPORTED_FILE=` to turn it off.

## Uninstall

```bash
docker compose down -v && rm -rf secrets .env                       # docker
sudo systemctl disable --now endlessh-reporter.socket endlessh-reporter && sudo rm -rf /opt/endlessh-reporter /etc/endlessh-reporter /etc/systemd/system/endlessh-reporter.service /etc/systemd/system/endlessh-reporter.socket /var/lib/private/endlessh-reporter /var/lib/endlessh-reporter   # systemd
```

The systemd line also deletes the event log. What the journal holds stays, under journald's own retention.

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

The tests run offline: unit tests plus end-to-end tests against local mock APIs and a loopback tarpit. CI additionally runs shellcheck on the installers, hadolint, a Trivy scan of the image and a daily `cargo audit`. Publishing an image needs the tests, `cargo audit` and the scan to pass.

Code map: `accept.rs` and `slots.rs` (taking connections, client limits), `tarpit.rs` (banner loop), `hello.rs` (is this an SSH client), `worker.rs` (filter, dedupe, fan out to sinks, event log), `sinks.rs` (the `Sink` trait), `abuseipdb.rs` and `webhook.rs` (the two sinks), `events.rs`, `ip.rs`, `config.rs`. Another API is one `impl Sink` plus a line in `main.rs`.

## License

MIT
