# Istio Ambient Benchmark Notes

Install Istio Ambient before applying `bench/workloads/echo.yaml`, then verify that the `istio-bench` namespace has `istio.io/dataplane-mode=ambient`.

The benchmark target is `http://echo.istio-bench.svc.cluster.local`.
