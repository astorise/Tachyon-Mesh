# Specifications: Zero-Trust Identity Injection

## 1. Cryptographic Keypair & Token
- On startup, the `core-host` generates an in-memory Ed25519 keypair. It shares the Public Key with all instantiated System FaaS via an environment variable or a specific WIT capability.
- The Identity Token is a lightweight JSON Web Token (JWT).
    
    Payload Example:
    {
        "sub": "faas-user-uuid-1234",
        "role": "user",
        "target_name": "image-processor",
        "iat": 1712050000,
        "exp": 1712050005
    }

*(Notice the expiration is just a few seconds, preventing replay attacks).*

## 2. Host Injection Mechanism
When a FaaS executes an outbound HTTP call via the Mesh:
1. The Host completely strips any existing `X-Tachyon-Identity` header provided by the user code (Preventing spoofing).
2. The Host generates the JWT, signs it with its private Ed25519 key, and injects it as `X-Tachyon-Identity: Bearer <token>`.

## 3. System FaaS ACL Enforcement
The `system-faas-storage-broker` receives the request.
1. It verifies the Ed25519 signature using the Host's Public Key.
2. It extracts the `target_name`.
3. It cross-references the requested file path against the `integrity.lock` configuration for that specific `target_name`. If the path is outside the allowed directory scope, it returns 403 Forbidden.