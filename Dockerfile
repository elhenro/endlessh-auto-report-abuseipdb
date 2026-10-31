# both bases are pinned by digest; dependabot proposes updates
FROM rust:1.92-alpine@sha256:f6c22e0a256c05d44fca23bf530120b5d4a6249a393734884281ca80782329bc AS build
# the final image has no shell to create the volume mount point with
RUN mkdir /data
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

# no libc, shell, perl or package manager: the musl binary is fully static.
# no ca certificates needed: the binary bundles its own roots
FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab

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
