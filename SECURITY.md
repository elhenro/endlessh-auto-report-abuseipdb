# Security

Report vulnerabilities privately via GitHub: **Security** tab, **Report a vulnerability**.
Please do not open public issues for security problems.

Design notes relevant to security:

- The tarpit never reads from a client, so nothing a client sends is ever parsed. Only the peer address is used, and it is a typed `IpAddr`, not text. The only network input parsed is the HTTPS response from AbuseIPDB or the webhook (status and headers; at most 200 bytes of an AbuseIPDB error body, logged with control characters blanked). `tests/security.rs` enforces the never-reads property.
- The AbuseIPDB token is held in memory and sent in one request header. It never appears in argv or in a child process environment (there are no child processes). API calls refuse redirects and plain http.
- The docker image is a static binary on distroless (no libc, shell or package manager), runs as `nobody` with a read-only root filesystem and no capabilities, and gets the token as a file so `docker inspect` does not show it. The systemd unit runs as a dynamic user with no capabilities (a socket unit owns the port), a syscall filter and `MemoryDenyWriteExecute`.
- Dependencies are limited to `ureq` (rustls), `url` (already pulled in by `ureq`) and `socket2`; dependabot tracks them, `Cargo.lock` is committed and CI runs `cargo audit` daily.
