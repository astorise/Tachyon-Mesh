# Specifications: Identity-Aware Connection Pipeline

## 1. System FaaS Auth (The Brain)
Create a new WIT file (`wit/identity.wit` or update `tachyon.wit`) with the following interface:

    interface auth {
        record claims {
            subject: string,
            roles: list<string>,
        }
        variant auth-error {
            expired,
            invalid-signature,
            missing-roles,
            internal-error(string),
        }
        verify-token: func(token: string, required-roles: list<string>) -> result<claims, auth-error>;
    }

The `system-faas-auth` crate must implement this interface. For this iteration, it must decode a JWT (using `jsonwebtoken` crate), verify it against a secret (injected via environment variable or default to "tachyon-dev-secret" for now), and ensure the extracted roles overlap with the required roles.

## 2. Core Host Middleware (The Enforcer)
The HTTP server in `core-host` must extract the `Authorization: Bearer <token>` header from incoming administrative requests (e.g., `/api/engine/status`). 
It must invoke the `verify-token` exported function of the instantiated `system-faas-auth` component, requesting the `"admin"` role. If the FaaS returns an error, the host must immediately return HTTP `401` or `403`.

## 3. Tachyon Client & UI (The Gateway)
The `tachyon-client` global state must hold `InstanceConfig { url: String, token: String, mtls_cert: Option<Vec<u8>>, mtls_key: Option<Vec<u8>> }`. 
The `tachyon-ui` must inject a full-screen overlay if no connection is active. The form must request:
- Node URL (String)
- Admin Token (String)
- mTLS Profile (File upload / bytes)