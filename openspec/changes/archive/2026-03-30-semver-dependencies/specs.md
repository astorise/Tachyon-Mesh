# Specifications: SemVer Architecture

## 1. Schema Update v5 (`integrity.lock`)
The configuration schema must include versions and constraints.

    {
        "targets": [
            {
                "name": "faas-a",
                "module": "faas-a-v2.wasm",
                "version": "2.0.0",
                "dependencies": {
                    "faas-b": "^3.1.0",
                    "legacy-db": "*"
                }
            },
            {
                "name": "faas-b",
                "module": "faas-b-v3-1.wasm",
                "version": "3.1.5",
                "dependencies": {}
            },
            {
                "name": "faas-b",
                "module": "faas-b-v4.wasm",
                "version": "4.0.0",
                "dependencies": {}
            }
        ]
    }

## 2. Startup Validation (`core-host`)
- Use the `semver::Version` and `semver::VersionReq` types.
- At boot, build a Registry: `HashMap<String, Vec<Target>>` (Key = Target Name, Value = List of available versions).
- Iterate through every target and its `dependencies`.
- For each dependency (e.g., `faas-b` matching `^3.1.0`), ensure that `Registry["faas-b"]` contains at least one target whose version matches the `VersionReq`. If not, abort startup.

## 3. IPC Routing Resolution
- When a FaaS module initiates an outbound call via the Host, the Host identifies the calling module (the "Caller").
- The Host intercepts the URL (e.g., `http://tachyon/faas-b`).
- The Host looks up the Caller's dependencies, finds the constraint `^3.1.0`.
- The Host queries the Registry for `faas-b`, filters by `^3.1.0`, sorts descending, and picks the highest compatible target (in this case, `3.1.5`, ignoring `4.0.0`).
- The call is routed to the selected Wasmtime component.