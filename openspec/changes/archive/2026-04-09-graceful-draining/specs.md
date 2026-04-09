# Specifications: Graceful Draining Mechanism

## 1. The Version Switch
When a new configuration is loaded:
- The Host instantiates the new version (`v2`) and marks it as `Active`.
- The old version (`v1`) is marked as `Draining`. No new requests are routed to `v1`.

## 2. The Reference Counter
Each FaaS instance maintains an atomic `active_requests` counter. 
- The `v1` instance stays alive as long as `active_requests > 0`.
- Once the counter hits 0, the Host's Garbage Collector (Change 033) cleans up the instance.

## 3. Safety Timeout (The "Kill Switch")
A global `draining_timeout` (default: 30s) is applied. If an instance of `v1` is still alive after 30s (e.g., due to a hung request), it is forcibly terminated to free resources.