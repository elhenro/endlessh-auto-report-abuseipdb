# pinned by digest; dependabot proposes updates
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587

# hadolint ignore=DL3008
RUN apt-get update \
    && apt-get install -y --no-install-recommends endlessh curl ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir /data && chown nobody:nogroup /data

WORKDIR /app
COPY --chmod=755 tarpitReporter.sh ./
COPY --chmod=755 lib ./lib

ENV REPORTED_FILE=/data/reportedIps.txt
VOLUME /data
USER 65534:65534

# endlessh listens on 2222 (unprivileged); publish it as host:22
EXPOSE 2222

# port check via /proc, no connection, so it never shows up as a visitor
HEALTHCHECK --interval=60s --timeout=5s --start-period=20s \
  CMD ["sh", "-c", "grep -qiE \":$(printf '%04X' \"${TARPIT_PORT:-2222}\") [0-9A-F:]+ 0A \" /proc/net/tcp /proc/net/tcp6"]

CMD ["./tarpitReporter.sh"]
