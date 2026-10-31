# Security

Report vulnerabilities privately via GitHub: **Security** tab, **Report a vulnerability**.
Please do not open public issues for security problems.

Design notes relevant to security:

- The tarpit never reads from a client, so nothing a client sends is ever parsed. It uses the peer address, a typed `IpAddr` and not text, and peeks at the first 4 bytes to compare them with `SSH-`; those stay unread in the kernel. The only network input parsed is the HTTPS response from AbuseIPDB or the webhook (status and headers; at most 200 bytes of an AbuseIPDB error body, logged with control characters blanked). `tests/security.rs` enforces the never-reads property.
- The AbuseIPDB token is held in memory and sent in one request header. It never appears in argv or in a child process environment (there are no child processes). API calls refuse redirects and plain http.
- The docker image is a static binary on distroless (no libc, shell or package manager), runs as its own uid with a read-only root filesystem and no capabilities, and gets the token as a file so `docker inspect` does not show it. The systemd unit runs as a dynamic user with no capabilities (a socket unit owns the port), a syscall filter and `MemoryDenyWriteExecute`.
- Direct dependencies are `ureq` (rustls), `url` and `socket2`, plus `libc` for two constants; the lockfile holds 67 crates with everything they pull in. Dependabot tracks them, `Cargo.lock` is committed, CI runs `cargo audit` daily and before every image is published, and the binary records its crate list so image scanners can see them.
- Images are built without a shared cache and carry a signed build attestation: `gh attestation verify oci://ghcr.io/elhenro/endlessh-auto-report-abuseipdb:latest --owner elhenro`.
