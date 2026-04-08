# Specifications: Layer 4 Routing

## 1. TCP Port Mapping (`integrity.lock`)
The configuration introduces a new `layer4` block at the host level, mapping exposed host ports to internal target names.

    {
        "host": {
            "layer4": {
                "2222": "ssh-gateway",
                "5432": "pg-proxy"
            }
        }
    }

## 2. The WASM `inetd` Pattern (Stream Piping)
When a TCP connection arrives on port 2222:
- The Host does NOT parse the protocol.
- It instantiates the `ssh-gateway` FaaS.
- It splits the TCP connection into an asynchronous `reader` and `writer`.
- Using `wasi_common::pipe`, it wires the TCP `reader` to the instance's `stdin` (File Descriptor 0).
- It wires the instance's `stdout` (File Descriptor 1) to the TCP `writer`.

*(Note: To prevent conflicts with Change 050 Async Logging, FaaS targets bound to Layer 4 must strictly use `stderr` for logging, as `stdout` is dedicated to the binary TCP protocol).*

## 3. Scale-to-Zero Lifecycle
A TCP connection is long-lived. 
- The FaaS execution loop (e.g., in Rust: `loop { std::io::stdin().read_exact(...) }`) blocks while waiting for bytes.
- When the remote client disconnects, the Host receives an `EOF` on the TCP socket.
- The Host closes the WASI `stdin` pipe. The FaaS loop breaks, the `main()` function returns, and the Host safely drops the instance.