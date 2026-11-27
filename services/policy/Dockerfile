FROM rust:1.100.0-alpine3.24 AS builder
WORKDIR /src
COPY . .
RUN for manifest in services/*/Cargo.toml; do \
        mkdir -p "${manifest%/Cargo.toml}/src" && \
        touch -a "${manifest%/Cargo.toml}/src/main.rs"; \
    done && \
    cargo build --release --locked --package policy

FROM scratch
COPY --from=builder /src/target/release/policy /policy
ENTRYPOINT ["/policy"]
