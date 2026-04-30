#!/usr/bin/env bash
set -euo pipefail

RESULT_DIR="${RESULT_DIR:-bench/results/raw}"
mkdir -p "$RESULT_DIR"

kubectl top pods --all-namespaces > "$RESULT_DIR/pod-resources.txt"
kubectl get pods --all-namespaces \
  -o custom-columns='NAMESPACE:.metadata.namespace,NAME:.metadata.name,READY:.status.containerStatuses[*].ready,RESTARTS:.status.containerStatuses[*].restartCount,NODE:.spec.nodeName' \
  > "$RESULT_DIR/pod-inventory.txt"
