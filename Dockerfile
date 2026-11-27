FROM rust:1.100.0-alpine3.24 AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY services/check/Cargo.toml services/check/
COPY services/dashboard/Cargo.toml services/dashboard/
COPY services/policy/Cargo.toml services/policy/
COPY services/ratio/Cargo.toml services/ratio/
COPY services/ups/Cargo.toml services/ups/
RUN for manifest in services/*/Cargo.toml; do \
        mkdir -p "${manifest%/Cargo.toml}/src" && \
        echo 'fn main() {}' > "${manifest%/Cargo.toml}/src/main.rs"; \
    done && \
    cargo build --release --locked --workspace
COPY services services
RUN touch services/*/src/main.rs && \
    cargo build --release --locked --workspace

FROM scratch AS check
COPY --from=builder /src/target/release/check /check
ENTRYPOINT ["/check"]

FROM scratch AS dashboard
COPY --from=builder /src/target/release/dashboard /dashboard
ENTRYPOINT ["/dashboard"]

FROM scratch AS policy
COPY --from=builder /src/target/release/policy /policy
ENTRYPOINT ["/policy"]

FROM scratch AS ratio
COPY --from=builder /src/target/release/ratio /ratio
ENTRYPOINT ["/ratio"]

FROM scratch AS ups
COPY --from=builder /src/target/release/ups /ups
ENTRYPOINT ["/ups"]
