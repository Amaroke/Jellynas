FROM caddy:2.11.7-builder-alpine AS builder
RUN xcaddy build \
    --with github.com/caddy-dns/ovh@v1.1.0 \
    --with github.com/mholt/caddy-dynamicdns@67d107a42c02

FROM caddy:2.11.7-alpine
COPY --from=builder /usr/bin/caddy /usr/bin/caddy
