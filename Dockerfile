# both bases are pinned by digest; dependabot proposes updates
FROM rust:1.92-alpine@sha256:f6c22e0a256c05d44fca23bf530120b5d4a6249a393734884281ca80782329bc AS build
SHELL ["/bin/ash", "-eo", "pipefail", "-c"]
# /data and the old entry point: the final image has no shell to create them with.
# cargo-auditable records the crate list in the binary: to an image scanner a stripped rust binary is blank otherwise
RUN mkdir /data /app && ln -s /usr/local/bin/endlessh-reporter /app/tarpitReporter.sh \
  && cargo install cargo-auditable --locked --version 0.7.7
COPY docker/static-pie-cc /usr/local/bin/static-pie-cc
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN RUSTFLAGS="-C linker=static-pie-cc -C relocation-model=pic" cargo auditable build --release --locked \
  && readelf -h target/release/endlessh-reporter | grep -q 'Position-Independent'

# no libc, shell, perl or package manager: the musl binary is fully static.
# no ca certificates needed: the binary bundles its own roots
FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab

COPY --from=build --chown=65534:65534 /data /data
COPY --from=build /app /app
COPY --from=build /src/target/release/endlessh-reporter /usr/local/bin/endlessh-reporter

WORKDIR /app
ENV REPORTED_FILE=/data/reportedIps.txt
VOLUME /data
# the old image's uid: existing `reported` volumes belong to it, and going back to the old image must keep
# working. a host service under this uid cannot look inside: the binary makes itself non-dumpable, see harden.rs
USER 65534:65534

# listens on 2222 (unprivileged); publish it as host:22
EXPOSE 2222

# port check via /proc, no connection, so it never shows up as a visitor
HEALTHCHECK --interval=60s --timeout=5s --start-period=20s \
  CMD ["/usr/local/bin/endlessh-reporter", "--healthcheck"]

CMD ["/usr/local/bin/endlessh-reporter"]
