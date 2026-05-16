FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
         endlessh curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY tarpitReporter.sh report.sh cache.sh endlessh-config-example.conf ./
RUN chmod +x tarpitReporter.sh report.sh cache.sh

# endlessh inside listens on 2222 (unprivileged); map host:22 -> container:2222
EXPOSE 2222

CMD ["./tarpitReporter.sh"]
