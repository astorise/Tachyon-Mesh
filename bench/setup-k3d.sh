#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${CLUSTER_NAME:-tachyon-bench}"
K3S_IMAGE="${K3S_IMAGE:-rancher/k3s:v1.30.6-k3s1}"

if ! command -v k3d >/dev/null 2>&1; then
  echo "k3d is required" >&2
  exit 127
fi

if k3d cluster list "$CLUSTER_NAME" >/dev/null 2>&1; then
  k3d cluster delete "$CLUSTER_NAME"
fi

k3d cluster create "$CLUSTER_NAME" \
  --image "$K3S_IMAGE" \
  --agents 2 \
  --servers 1 \
  --wait

kubectl create namespace tachyon-bench --dry-run=client -o yaml | kubectl apply -f -
kubectl create namespace istio-bench --dry-run=client -o yaml | kubectl apply -f -
kubectl create namespace linkerd-bench --dry-run=client -o yaml | kubectl apply -f -

kubectl label namespace istio-bench istio.io/dataplane-mode=ambient --overwrite
kubectl label namespace linkerd-bench linkerd.io/inject=enabled --overwrite
