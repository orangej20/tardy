# syntax=docker/dockerfile:1.7
FROM rust:1.88-alpine3.22 AS builder

RUN apk add --no-cache build-base cmake perl
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY ingest ./ingest
COPY migrations ./migrations

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --locked --release --bins && \
    cp target/release/tardy /tmp/tardy && \
    cp target/release/ingest-worker /tmp/tardy-ingest-worker

FROM alpine:3.22

RUN apk add --no-cache ca-certificates && \
    addgroup -S tardy && \
    adduser -S -D -H -G tardy tardy && \
    mkdir -p /data && \
    chown tardy:tardy /data

COPY --from=builder /tmp/tardy /usr/local/bin/tardy
COPY --from=builder /tmp/tardy-ingest-worker /usr/local/bin/tardy-ingest-worker

USER tardy:tardy
WORKDIR /data
ENV TARDY_BIND=0.0.0.0:3000 \
    TARDY_PUBLIC_BASE_URL=http://localhost:3000 \
    TARDY_DB_PATH=/data/tardy.sqlite \
    RUST_LOG=info
EXPOSE 3000
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q -T 2 -O /dev/null http://127.0.0.1:3000/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/tardy"]
