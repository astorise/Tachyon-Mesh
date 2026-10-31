# HTTP/3 interoperability

The `quic-interop` job in the manually dispatched Integration workflow starts
Tachyon's HTTP/3 listener on loopback and uses the independent, pinned aioquic
client. It verifies a TLS 1.3 handshake, ALPN `h3`, the server certificate,
and three concurrent requests and responses on one QUIC connection. A second
attempt must fail when the client trusts the wrong CA.

To run the same test locally with Python 3.12 and the Rust WASI target installed:

```sh
python3.12 -m venv /tmp/tachyon-aioquic
/tmp/tachyon-aioquic/bin/pip install --require-hashes -r tests/conformance/quic/requirements.txt
cargo build --locked -p guest-example --target wasm32-wasip2
TACHYON_AIOQUIC_PYTHON=/tmp/tachyon-aioquic/bin/python \
  cargo test --locked -p core-host --bin core-host --features http3 \
  host_core::tests::l4_tls_quic::aioquic_http3_interoperability -- --ignored --exact
```

The pinned Python dependency hashes target Linux x86_64 and Python 3.12, as
used by CI. This test does not cover QUIC Retry, 0-RTT, or the complete
[QUIC interop runner](https://github.com/marten-seemann/quic-interop-runner)
suite. The latter requires a dedicated server adapter for its HTTP/0.9 test
cases; Tachyon's endpoint serves HTTP/3.
