# both bases are pinned by digest; dependabot proposes updates
FROM rust:1.92-slim-bookworm@sha256:f1f73538ebe623fd3673a35aff3df358ae1084c64c55646516e5b17b321b6c9b AS build
# the final image has no shell to create the volume mount point with
RUN mkdir /data
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

# no shell, perl or package manager for an intruder to use; same bookworm glibc as the build.
# no ca certificates needed: the binary bundles its own roots
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

COPY --from=build --chown=65534:65534 /data /data
COPY --from=build /src/target/release/endlessh-reporter /usr/local/bin/endlessh-reporter

VOLUME /data
USER 65534:65534

# listens on 2222 (unprivileged); publish it as host:22
EXPOSE 2222

# port check via /proc, no connection, so it never shows up as a visitor
HEALTHCHECK --interval=60s --timeout=5s --start-period=20s \
  CMD ["/usr/local/bin/endlessh-reporter", "--healthcheck"]

CMD ["/usr/local/bin/endlessh-reporter"]
