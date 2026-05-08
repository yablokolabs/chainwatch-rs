FROM rust:1.94-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /nonexistent --shell /usr/sbin/nologin chainwatch
WORKDIR /app
COPY --from=builder /app/target/release/chainwatch-rs /usr/local/bin/chainwatch-rs
USER chainwatch
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/chainwatch-rs"]
