# Technical Specification: GPU Homelab Manifest

## 1. Advanced Manifest (`manifests/deploy-gpu-homelab.yaml`)
Create a comprehensive Kubernetes manifest optimized for local AI clusters.

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: tachyon-core-host
spec:
  replicas: 1
  template:
    spec:
      nodeSelector:
        [nvidia.com/gpu](https://nvidia.com/gpu): "present"  # Force scheduling on GPU nodes
      containers:
      - name: core-host
        image: astorise/tachyon-mesh:latest
        resources:
          limits:
            [nvidia.com/gpu](https://nvidia.com/gpu): 1      # Request GPU access
            memory: "8Gi"
          requests:
            memory: "4Gi"
        volumeMounts:
        - name: model-cache
          mountPath: /var/lib/tachyon/models
      volumes:
      - name: model-cache
        persistentVolumeClaim:
          claimName: tachyon-model-pvc
---
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: tachyon-model-pvc
spec:
  accessModes:
    - ReadWriteOnce
  resources:
    requests:
      storage: 50Gi
---
# Add basic RBAC rules if the host needs to query the K8s API
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
# ...
```