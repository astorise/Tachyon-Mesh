# Technical Specification: Hardened K8s Manifest

## 1. Create `manifests/deploy-gpu-homelab-hardened.yaml`
Assemble the secure primitives. 

### Part A: Identity (ServiceAccount)
```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: tachyon-sa
  namespace: default
```

### Part B: The Hardened Deployment
Update the deployment spec to enforce the Restricted Pod Security Standard.
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: tachyon-core-host
spec:
  template:
    spec:
      serviceAccountName: tachyon-sa
      securityContext:
        runAsNonRoot: true
        runAsUser: 10001
        runAsGroup: 10001
        fsGroup: 10001
        seccompProfile:
          type: RuntimeDefault
      containers:
      - name: core-host
        image: astorise/tachyon-mesh:latest
        securityContext:
          allowPrivilegeEscalation: false
          readOnlyRootFilesystem: true
          capabilities:
            drop:
              - ALL
        volumeMounts:
        - name: tmp-vol
          mountPath: /tmp
        - name: model-cache
          mountPath: /var/lib/tachyon/models
      volumes:
      - name: tmp-vol
        emptyDir: {}
```
*(Note: Because the root filesystem is read-only, we must mount an `emptyDir` to `/tmp` if the application needs to write temporary files).*

### Part C: Network Isolation (NetworkPolicy)
Create a zero-trust network perimeter.
```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: tachyon-network-policy
spec:
  podSelector:
    matchLabels:
      app: tachyon-core-host
  policyTypes:
  - Ingress
  - Egress
  ingress:
  - ports:
    - protocol: TCP
      port: 8080 # App API & MCP
    - protocol: TCP
      port: 9090 # Metrics
  egress:
  - ports:
    - protocol: UDP
      port: 53 # DNS
    - protocol: TCP
      port: 443 # K8s API & External Artifact fetch
```