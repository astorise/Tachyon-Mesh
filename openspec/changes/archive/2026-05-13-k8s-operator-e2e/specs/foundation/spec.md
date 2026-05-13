# Technical Specification: K8s E2E Workflow

## 1. GitHub Action Workflow (`.github/workflows/e2e-k8s.yml`)
Create a new workflow dedicated to testing the Kubernetes operator path.

```yaml
name: E2E K8s Operator (vcluster)

on:
  push:
    branches: [ main ]
    paths:
      - 'manifests/**'
      - 'core-host/**'
      - 'Dockerfile'
  pull_request:
    paths:
      - 'manifests/**'

jobs:
  test-k8s-deployment:
    runs-on: ubuntu-latest
    steps:
      - name: Checkout code
        uses: actions/checkout@v4

      - name: Setup k3d/k3s
        uses: nolar/setup-k3d-k3s@v1
        with:
          version: v1.28
          k3d-name: tachyon-host-cluster

      - name: Install vcluster CLI
        run: |
          curl -L -o vcluster "[https://github.com/loft-sh/vcluster/releases/latest/download/vcluster-linux-amd64](https://github.com/loft-sh/vcluster/releases/latest/download/vcluster-linux-amd64)"
          chmod +x vcluster
          sudo mv vcluster /usr/local/bin/

      - name: Create vcluster environment
        run: |
          vcluster create tachyon-vcluster --connect=false
          vcluster connect tachyon-vcluster
          
          # Verify connection is pointing to the virtual cluster
          kubectl get namespaces

      - name: Apply Deploy Manifest
        run: |
          # Note: In a real PR, you might want to dynamically build the local Docker image 
          # and load it into k3d, patching the manifest to use the local image instead of 'latest'.
          # For testing the vanilla operator path, applying the raw manifest is standard.
          kubectl apply -f manifests/deploy.yaml

      - name: Wait for Core-Host Readiness
        run: |
          echo "Waiting for tachyon-core-host pods to be ready..."
          # Adjust the label selector based on what is actually in deploy.yaml
          kubectl wait --for=condition=ready pod -l app=tachyon-core-host --timeout=120s

      - name: Basic API Healthcheck
        run: |
          # Forward port locally
          kubectl port-forward svc/tachyon-core-host 8080:8080 &
          sleep 5
          # Assert health endpoint
          curl -f http://localhost:8080/health || exit 1
```

## 2. Docker Image Injection (Optional but recommended for PRs)
If testing a PR, the manifest needs the *newly built* image, not the one from the public registry.
We must add a step to build the local `Dockerfile`, import it via `k3d image import`, and override the deployment image using `sed` or `kustomize` before applying it to the `vcluster`.