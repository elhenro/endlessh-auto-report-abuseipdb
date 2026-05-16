# endlessh auto-report to AbuseIPDB

SSH tarpit ([endlessh](https://github.com/skeeto/endlessh)) that stalls bot login attempts and reports the source IPs to [AbuseIPDB](https://www.abuseipdb.com/).

Each IP is reported at most once per 15 minutes (local cache).

## Requirements

- [endlessh](https://github.com/skeeto/endlessh)
- [curl](https://github.com/curl/curl)
- An AbuseIPDB account + API token (free tier: 1000 reports / day)

## Bare-metal usage

1. Move your real `sshd` to a non-standard port (anywhere between 1024 and 65535) so endlessh can bind `:22`.
2. `export API_TOKEN=your-abuseipdb-token`
3. `./tarpitReporter.sh`

Reported IPs are appended to `reportedIps.txt`.

## Docker usage

```bash
cp .env.example .env
$EDITOR .env                      # set API_TOKEN
docker compose up -d --build
docker compose logs -f
```

The compose file maps host `:22` to container `:2222` (endlessh runs unprivileged inside).
Make sure the host's real `sshd` is on a different port first, or the bind will fail.

To stop:

```bash
docker compose down
```

## Notes

- The bind family is `0` (IPv4 + IPv6). IPv6 attacker addresses are reported as-is.
- Logs rotate at 10 MB × 3 files (configured in `docker-compose.yml`).
- `reportedIps.txt` is for your own audit; trim or rotate as needed.
