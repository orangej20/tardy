#[test]
fn dockerfile_keeps_the_runtime_minimal_and_non_root() {
    let dockerfile = include_str!("../Dockerfile");
    assert!(dockerfile.contains("FROM rust:1.88-alpine3.22 AS builder"));
    assert!(dockerfile.contains("cargo build --locked --release"));
    assert!(dockerfile.contains("apk add --no-cache build-base cmake perl"));
    assert!(dockerfile.contains("FROM alpine:3.22"));
    assert!(dockerfile.contains("USER tardy:tardy"));
    assert!(dockerfile.contains("TARDY_DB_PATH=/data/tardy.sqlite"));
    assert!(dockerfile.contains("HEALTHCHECK"));
    assert!(dockerfile.contains("/healthz"));
    assert!(!dockerfile.contains("FROM ubuntu"));
    assert!(!dockerfile.contains("FROM debian"));
}
