# Specifications: TLS Termination & Cert-Manager

## 1. Domain Configuration (`integrity.lock`)
Targets can now explicitly declare their custom domains.

    {
        "targets": [
            {
                "name": "production-api",
                "module": "api.wasm",
                "domains": ["api.my-startup.com", "app.my-startup.com"]
            }
        ]
    }

## 2. Dynamic SNI & TLS Offloading
When a connection arrives on port 443 (HTTPS) or a TLS-wrapped Layer 4 port:
- The Host reads the SNI (Server Name Indication) from the TLS `ClientHello` packet before establishing the encrypted tunnel.
- The Host queries its in-memory `CertificateCache`.
- **Cache Hit:** The Host completes the TLS handshake, decrypts the stream, and passes pure Cleartext (HTTP/TCP) to the local FaaS router.
- **Cache Miss:** The Host suspends the TLS handshake task and delegates to the `system-faas-cert-manager`.

## 3. The Cert-Manager Lifecycle
The `system-faas-cert-manager` acts as the ACME client:
1. It receives the domain name from the Host.
2. It initiates an ACME order with Let's Encrypt.
3. It handles the `HTTP-01` challenge by temporarily injecting a validation route into the Host's router.
4. Once validated, it downloads the certificate and private key.
5. It uses the `system-faas-storage-broker` (Change 040) to persist the certificate to disk, preventing rate limits on node restarts.
6. It returns the certificate to the Host, which updates its RAM cache and completes the suspended TLS handshake.