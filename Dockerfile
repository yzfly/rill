FROM rust:1.98-slim-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --locked --release -p rill-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/rill /usr/local/bin/rill
USER 65532:65532
ENV PORT=8787
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/rill"]
