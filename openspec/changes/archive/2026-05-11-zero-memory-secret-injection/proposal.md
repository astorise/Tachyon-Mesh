# Proposal: Change 078 - Zero-Memory Secret Injection (WASI Interceptor)

## Context
Currently, when a FaaS module requires access to external APIs (e.g., OpenAI, AWS, Stripe), secrets are passed via environment variables and reside in the linear memory of the WebAssembly instance. If a guest module is compromised (via an internal vulnerability or malicious dependency), the attacker can dump the memory and exfiltrate the plaintext secrets. Inspired by projects like Kloak, we can eradicate this attack vector entirely. Because Tachyon Mesh uses Wasmtime and explicitly handles outbound HTTP via the `wasi:http` interface, the `core-host` can act as a cryptographic proxy.

## Objective
Implement "Zero-Memory Secret Injection". Guest WASM modules will only ever receive dummy placeholders (e.g., `tachyon:secret:uuid`). When the guest makes an outbound HTTP request, the `core-host` will intercept it at the WASI boundary, verify the destination domain (Egress Control), and swap the placeholder with the real secret right before writing to the network socket.

## Scope
1. Define the Placeholder Token format.
2. Implement an Egress Secret Store in the `core-host`.
3. Create a WASI HTTP Outgoing Request middleware to intercept, scan, and rewrite HTTP headers/bodies.