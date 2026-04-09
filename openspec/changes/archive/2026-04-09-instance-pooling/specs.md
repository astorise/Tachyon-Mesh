# Specifications: Instance Pooling Architecture

## 1. Schema Update (`integrity.lock`)
Add the `scale` configuration block to targets. If omitted, default values apply (e.g., `min: 0`, `max: 1000`).

    {
        "targets": [
            {
                "name": "payment-gateway",
                "module": "stripe-processor.wasm",
                "role": "user",
                "scale": {
                    "min": 5,
                    "max": 50
                }
            },
            {
                "name": "system-storage",
                "module": "system-faas-storage-broker.wasm",
                "role": "system",
                "scale": {
                    "max": 1
                }
            }
        ]
    }

## 2. Pool Mechanics
- **Pre-warming:** During host startup or hot-reload (Change 026), the host loops `min` times to compile and instantiate the module, pushing the ready-to-use instances into the pool.
- **Acquisition:** When an HTTP request arrives, the router attempts to `pop()` an instance from the pool.
  - If the pool is empty AND `current_total < max`: the host instantiates a new module on the fly, passes the request, and increments `current_total`.
  - If the pool is empty AND `current_total == max`: the router `awaits` asynchronously on a notification channel (Queue).
- **Release:** When a WASM instance finishes processing the HTTP response, the host cleans its memory (calling any export cleanup if necessary) and pushes the instance back into the pool, notifying the next waiting request in the queue.