# Benchmark Profile

Public baseline profile:

- Host class: AWS c6i.xlarge or equivalent x86_64
- Cluster: k3d, one server, two agents
- Load generator: Fortio from a host with no colocated benchmark target pods
- Duration: 60 seconds per target and QPS level
- QPS levels: 1,000 and 10,000
- Connections: 64

Update this file with the exact cloud region, kernel, Docker, k3d, Kubernetes, Istio, Linkerd, and Tachyon revisions before publishing real numbers.
