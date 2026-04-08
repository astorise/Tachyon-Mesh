# Specifications: Auth Middleware & Credential Graph

## 1. Schema Update v7 (`integrity.lock`)
The configuration must support middlewares and required credentials.

    {
        "targets": [
            {
                "name": "system-faas-auth",
                "module": "auth-zanzibar.wasm",
                "version": "1.0.0",
                "role": "system"
            },
            {
                "name": "faas-b",
                "module": "faas-b-v2.wasm",
                "version": "2.0.0",
                "requires_credentials": ["c2"],
                "dependencies": {}
            },
            {
                "name": "faas-a",
                "module": "faas-a-v1.wasm",
                "version": "1.0.0",
                "middleware": "system-faas-auth",
                "requires_credentials": ["c1", "c2"], // A needs c1 for itself, and c2 to call B
                "dependencies": {
                    "faas-b": "^2.0.0"
                }
            }
        ]
    }

## 2. Deploy-Time Credential Graph Validation (`core-host`)
Extend the validation logic from Change 025:
- After validating SemVer, build a strict credential tree.
- For each module, check its `dependencies`.
- If `faas-a` calls `faas-b` (version 2.0.0), look at `faas-b`'s `requires_credentials` (which contains `c2`).
- Verify that `faas-a`'s `requires_credentials` is a **superset** that includes `c2` (or explicitly defines a delegation map).
- If `c2` is missing from `faas-a`, abort startup: `Panic: Missing delegated credential 'c2' in FaaS-A to satisfy FaaS-B`.

## 3. Middleware Execution Flow (Run-Time)
- In the Axum `faas_handler`, check if the selected target has a `middleware` defined.
- If yes, load and execute the middleware WASM component first.
- The middleware component MUST return a standard HTTP response.
- **Contract:** If the middleware returns HTTP `200 OK`, the Host extracts the headers/context, discards the 200 body, and proceeds to load and execute the main target FaaS.
- If the middleware returns anything else (e.g., `401`, `403`, `302`), the Host immediately returns that response to the client and halts the execution chain.