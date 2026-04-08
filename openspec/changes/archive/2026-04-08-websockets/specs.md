# Specifications: WebSocket Architecture

## 1. WIT Interface Updates (`wit/tachyon.wit`)
We must define the bidirectional channel as a Resource.

    package tachyon:mesh;

    interface websocket {
        variant frame {
            text(string),
            binary(list<u8>),
            ping(list<u8>),
            pong(list<u8>),
            close,
        }

        resource connection {
            // Sends a frame to the client
            send: func(msg: frame) -> result<_, string>;
            
            // Blocks/Suspends until a frame is received from the client
            receive: func() -> result<frame, string>;
        }

        // The entrypoint for the guest FaaS
        export on-connect: func(conn: connection);
    }

## 2. Configuration Schema (`integrity.lock`)
Routes must explicitly declare if they expect a WebSocket upgrade.

    {
        "targets": [
            {
                "name": "chat-server",
                "module": "guest-chat.wasm",
                "websocket": true
            }
        ]
    }

## 3. Host Implementation (`core-host`)
Behind `#[cfg(feature = "websockets")]`:
- Use `axum::extract::ws::{WebSocketUpgrade, WebSocket}`.
- In the handler, if `config.websocket == true`, return `ws_upgrade.on_upgrade(|socket| handle_socket(socket, target))`.
- `handle_socket` does the following:
  1. Wraps the Axum `socket` (which is a Stream/Sink) inside a Rust struct.
  2. Inserts this struct into the Wasmtime `ResourceTable`.
  3. Instantiates the WASM component.
  4. Calls the exported `on_connect` function, passing the Resource ID.
  5. Under the hood, when the WASM calls `receive()`, the Rust host calls `.next().await` on the socket. Because we use async Wasmtime, this suspends the WASM execution efficiently without blocking the OS thread.