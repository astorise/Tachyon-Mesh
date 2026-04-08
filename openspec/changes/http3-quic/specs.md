# Specifications: HTTP/3 Architecture

## 1. Feature Flag Definition (`core-host/Cargo.toml`)
We rely on the Tokio-backed `quinn` implementation.

    [features]
    default = []
    http3 = ["quinn", "h3", "h3-quinn"]

    [dependencies]
    quinn = { version = "0.11", optional = true }
    h3 = { version = "0.0.4", optional = true }
    h3-quinn = { version = "0.0.5", optional = true }

## 2. Conditional UDP Listener (`core-host`)
HTTP/3 mandates TLS 1.3. The host MUST reuse the certificates defined in Change 029 (mTLS/TLS setup).

    #[cfg(feature = "http3")]
    pub async fn spawn_http3_server(app: axum::Router, addr: std::net::SocketAddr, certs: TlsConfig) {
        // 1. Configure Quinn ServerConfig with TLS 1.3 certs
        let server_config = quinn::ServerConfig::with_crypto(rustls_config);
        
        // 2. Bind UDP Socket
        let endpoint = quinn::Endpoint::server(server_config, addr).unwrap();
        
        // 3. Accept Loop
        while let Some(incoming) = endpoint.accept().await {
            let app = app.clone();
            tokio::spawn(async move {
                let connection = incoming.await.unwrap();
                let mut h3_conn = h3::server::Connection::new(h3_quinn::Connection::new(connection)).await.unwrap();
                
                // 4. Demultiplex streams and pass to Axum (Tower Service)
                while let Some((req, mut stream)) = h3_conn.accept().await.unwrap() {
                    let app = app.clone();
                    tokio::spawn(async move {
                        // Bridge between h3 request and Axum tower::Service implementation
                        let response = app.oneshot(req).await.unwrap();
                        stream.send_response(response).await.unwrap();
                    });
                }
            });
        }
    }

## 3. Graceful Coexistence
If both TCP and UDP are enabled, the Host spawns two separate `tokio::task` loops. The internal WASM modules never know if the request arrived via TCP/HTTP2 or UDP/HTTP3. The abstraction is perfect.