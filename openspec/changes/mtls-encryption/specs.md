# Specifications: System FaaS Gateway Architecture

## 1. WIT Interface Updates (`wit/tachyon.wit`)
We must define how the WASM guest asks the Host to perform TLS.

    package tachyon:mesh;

    interface tls-crypto {
        // Represents a handle to an active TLS session managed by the host
        resource tls-session {
            // Read decrypted bytes
            read: func(max-bytes: u32) -> list<u8>;
            // Write bytes to be encrypted and sent
            write: func(data: list<u8>) -> u32;
        }

        // The FaaS passes a raw WASI TCP socket descriptor. 
        // The Host performs the mTLS handshake using its native certs.
        // Returns a secure tls-session resource.
        upgrade-to-mtls: func(socket-fd: u32) -> result<tls-session, string>;
    }

## 2. Host Capability Implementation (`core-host`)
Behind `#[cfg(feature = "mtls")]`:
- The Host implements the `tls-crypto` trait.
- `upgrade_to_mtls` takes the file descriptor, converts it into a `tokio::net::TcpStream`, and wraps it using `tokio_rustls::TlsAcceptor`.
- It stores the resulting `TlsStream` in the `WasiCtx` Resource Table and returns the handle ID (`tls-session`) to the WASM module.
- `read` and `write` simply call `AsyncReadExt` and `AsyncWriteExt` on the stored `TlsStream`.

## 3. The Gateway FaaS (`system-faas-gateway`)
This is an infinite-loop background WASM component (like the autoscaler from Change 021).
- It uses `wasi::sockets::tcp` to bind to `0.0.0.0:8443` and call `listen()`.
- On `accept()`, it calls `tachyon::crypto::tls::upgrade_to_mtls(client_socket)`.
- If the handshake fails (e.g., the client has no valid certificate), it drops the connection.
- If successful, it reads the HTTP request bytes from the `tls-session`, parses the HTTP headers, and makes an internal mesh call to the appropriate business FaaS.