# syntax=docker/dockerfile:1.7
# Multi-stage Rust build → slim runtime image.
# Built and pushed by .github/workflows/release.yml to ghcr.io/me1iissa/auditnetwork.

FROM rust:1.94-bookworm AS builder
WORKDIR /build

# Cache deps separately from sources.
COPY Cargo.toml Cargo.lock ./
COPY crates/cli/Cargo.toml crates/cli/Cargo.toml
COPY crates/ingest/Cargo.toml crates/ingest/Cargo.toml
COPY crates/store/Cargo.toml crates/store/Cargo.toml
COPY crates/model/Cargo.toml crates/model/Cargo.toml
RUN mkdir -p crates/cli/src crates/ingest/src crates/store/src crates/model/src \
 && echo 'fn main(){}' > crates/cli/src/main.rs \
 && echo '' > crates/ingest/src/lib.rs \
 && echo '' > crates/store/src/lib.rs \
 && echo '' > crates/model/src/lib.rs \
 && cargo build --release -p cli || true

COPY migrations migrations
COPY crates crates
RUN cargo build --release -p cli --locked

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && rm -rf /var/lib/apt/lists/*
RUN useradd -m -s /usr/sbin/nologin -u 10001 auditnetwork \
 && mkdir -p /data \
 && chown auditnetwork:auditnetwork /data
USER auditnetwork
WORKDIR /home/auditnetwork
COPY --from=builder /build/target/release/auditnetwork /usr/local/bin/auditnetwork
ENV AN_DB=/data/audit.db
ENV AN_BIND=0.0.0.0:8080
ENV RUST_LOG=info
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["/usr/bin/tini", "--", "auditnetwork"]
CMD ["serve"]
