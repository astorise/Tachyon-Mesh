# Linkerd Benchmark Notes

Install Linkerd before applying `bench/workloads/echo.yaml`, then verify that the `linkerd-bench` namespace has `linkerd.io/inject=enabled`.

The benchmark target is `http://echo.linkerd-bench.svc.cluster.local`.
