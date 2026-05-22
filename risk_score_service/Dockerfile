# zarrs 0.23+ needs a very recent stable Rust (see crates.io rustc version requirements)
FROM rust:latest AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/risk_score_worker /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/risk_score_worker"]
