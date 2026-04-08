# Proposal: Change 057 - Native Edge TLS & Automated ACME

## Context
A commercial PaaS must serve traffic over HTTPS securely. Forcing User FaaS to handle TLS encryption internally would destroy CPU performance (WebAssembly cryptography overhead) and ruin the developer experience. Conversely, placing an external reverse proxy (like Nginx) in front of Tachyon breaks the "Single Binary" architecture and loses mesh-awareness. The `core-host` must natively terminate TLS and automatically manage SSL certificates for custom domains.

## Objective
1. Integrate an async, high-performance TLS terminator (e.g., `rustls`) directly into the `core-host` network listener.
2. Implement SNI (Server Name Indication) routing to map incoming encrypted traffic to the correct FaaS target without decrypting the payload inside the WASM sandbox.
3. Create a `system-faas-cert-manager` responsible for negotiating, renewing, and storing Let's Encrypt (ACME) certificates on the fly.

## Scope
- Add a new `domains` array to the target configuration in `integrity.lock`.
- Implement dynamic certificate resolution in the Tokio TCP/HTTP pipeline.
- Implement the ACME HTTP-01 or TLS-ALPN-01 challenge logic in the System FaaS.

## Success Metrics
- A developer adds `api.startup.com` to their target in `integrity.lock` and points their DNS to the Tachyon node.
- On the very first incoming HTTPS request, the host pauses the connection, fetches a Let's Encrypt certificate in <2 seconds, resumes the connection, and serves the request securely. Subsequent requests take <1ms.