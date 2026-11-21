FROM rust:1.100.0-alpine3.24 AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM scratch
COPY --from=builder /src/target/release/jellyfin-policy /jellyfin-policy
ENTRYPOINT ["/jellyfin-policy"]
