# Specifications: Storage Broker Architecture

## 1. Schema Update (`integrity.lock`)
Volume mounts must explicitly declare their access mode. The Host will reject any `integrity.lock` where a `role: "user"` target attempts to request `mode: "rw"`.

    {
        "volumes": {
            "shared-data": { "type": "hostPath", "path": "/mnt/tachyon/data" }
        },
        "targets": [
            {
                "name": "image-processor",
                "module": "user-faas.wasm",
                "role": "user",
                "mounts": [{ "volume": "shared-data", "path": "/data", "mode": "ro" }]
            },
            {
                "name": "system-storage",
                "module": "system-faas-storage-broker.wasm",
                "role": "system",
                "singleton": true,
                "mounts": [{ "volume": "shared-data", "path": "/data", "mode": "rw" }]
            }
        ]
    }

## 2. The IPC Write Protocol
When a User FaaS needs to write to `/data/output.jpg`, it makes a Mesh IPC call:
- **Method:** POST
- **URL:** `http://mesh/system-storage/write?path=/data/output.jpg&mode=overwrite` (or `mode=append`)
- **Body:** The binary payload to write.

## 3. Broker Queue Mechanics
The `system-faas-storage-broker` receives the HTTP POST.
- It immediately pushes the write task (path, mode, payload) into an internal memory queue (e.g., a Rust `VecDeque`).
- It returns an HTTP 202 Accepted to the User FaaS (non-blocking).
- A background asynchronous loop inside the Broker pops tasks from the queue one by one and uses `wasi:filesystem` to execute the writes sequentially on the OS, ensuring strict consistency.