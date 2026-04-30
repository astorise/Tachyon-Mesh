# QUIC Conformance Harness

This directory contains the CI entry point for the QUIC interop runner. The integration workflow pulls the official runner image and targets Tachyon's HTTP/3 wrapper for handshake, transfer, retry, stream multiplexing, and 0-RTT related regressions.

The local command is:

```bash
docker compose -f tests/conformance/quic/docker-compose.yml run --rm quic-interop-runner
```
