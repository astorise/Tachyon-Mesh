#!/usr/bin/env bash
set -euo pipefail

cargo build -p guest-ai --target wasm32-wasip1 --release
cargo build -p guest-example --target wasm32-wasip2 --release
cargo build -p guest-flaky --target wasm32-wasip2 --release
cargo build -p guest-grpc --target wasm32-wasip2 --release
cargo build -p guest-log-storm --target wasm32-wasip1 --release
cargo build -p guest-tcp-echo --target wasm32-wasip1 --release
cargo build -p guest-udp-echo --target wasm32-wasip2 --release
cargo build -p guest-voip-gate --target wasm32-wasip2 --release
cargo build -p guest-volume --target wasm32-wasip2 --release
cargo build -p system-faas-bridge --target wasm32-wasip2 --release
cargo build -p system-faas-authn --target wasm32-wasip2 --release
cargo build -p system-faas-authz --target wasm32-wasip2 --release
cargo build -p system-faas-cert-manager --target wasm32-wasip2 --release
cargo build -p system-faas-cdc --target wasm32-wasip2 --release
cargo build -p system-faas-buffer --target wasm32-wasip2 --release
cargo build -p system-faas-gc --target wasm32-wasip2 --release
cargo build -p system-faas-gateway --target wasm32-wasip2 --release
cargo build -p system-faas-gossip --target wasm32-wasip2 --release
cargo build -p system-faas-keda --target wasm32-wasip2 --release
cargo build -p system-faas-k8s-scaler --target wasm32-wasip2 --release
cargo build -p system-faas-logger --target wasm32-wasip2 --release
cargo build -p system-faas-model-broker --target wasm32-wasip2 --release
cargo build -p system-faas-metering --target wasm32-wasip2 --release
cargo build -p system-faas-prom --target wasm32-wasip2 --release
cargo build -p system-faas-registry --target wasm32-wasip2 --release
cargo build -p system-faas-s3-proxy --target wasm32-wasip2 --release
cargo build -p system-faas-shadow-proxy --target wasm32-wasip2 --release
cargo build -p system-faas-sqs --target wasm32-wasip2 --release
cargo build -p system-faas-storage-broker --target wasm32-wasip2 --release
cargo build -p guest-call-legacy --target wasm32-wasip1 --release
cargo build -p guest-loop --target wasm32-wasip1 --release
cargo build -p guest-malicious --target wasm32-wasip1 --release
