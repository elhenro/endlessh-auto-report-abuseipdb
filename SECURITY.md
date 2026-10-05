# Security

Report vulnerabilities privately via GitHub: **Security** tab, **Report a vulnerability**.
Please do not open public issues for security problems.

Design notes relevant to security:

- The AbuseIPDB token is passed to curl through a config file descriptor, never argv or an environment variable of child processes.
- The docker image runs as `nobody` with a read-only root filesystem and no capabilities; the systemd unit runs as a dynamic user with only `CAP_NET_BIND_SERVICE`.
- Only strictly validated IP addresses ever leave the log parser. Nothing from the network is passed to a shell.
