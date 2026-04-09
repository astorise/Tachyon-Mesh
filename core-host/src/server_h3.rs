use anyhow::{anyhow, Context, Result};
use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, Request},
    response::Response,
    Router,
};
use bytes::{Buf, BytesMut};
use h3::server::RequestResolver;
use http_body_util::BodyExt;
use quinn::crypto::rustls::QuicServerConfig;
use std::sync::Arc;
use tower::ServiceExt;

pub(crate) async fn start_http3_listener(
    state: crate::AppState,
    app: Router,
) -> Result<Option<crate::Http3ListenerHandle>> {
    let runtime = state.runtime.load_full();
    let Some(bind_address) = crate::https_bind_address(&runtime.config)? else {
        return Ok(None);
    };
    let domain = runtime
        .config
        .routes
        .iter()
        .flat_map(|route| route.domains.iter())
        .next()
        .cloned()
        .ok_or_else(|| anyhow!("HTTP/3 requires at least one sealed custom domain"))?;
    drop(runtime);

    let tls_config = state
        .tls_manager
        .server_config_for_domain(&state, &domain)
        .await
        .context("failed to provision TLS material for HTTP/3 listener")?;
    let server_config =
        build_quinn_server_config(tls_config.as_ref()).context("failed to build QUIC config")?;
    let endpoint = quinn::Endpoint::server(server_config, bind_address)
        .with_context(|| format!("failed to bind HTTP/3 listener on {bind_address}"))?;
    let local_addr = endpoint
        .local_addr()
        .context("failed to read HTTP/3 listener local address")?;

    let join_handle = tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let app = app.clone();
            tokio::spawn(async move {
                let connection = match incoming.await {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::warn!("HTTP/3 QUIC handshake failed: {error}");
                        return;
                    }
                };

                if let Err(error) = handle_http3_connection(app, connection).await {
                    tracing::warn!("HTTP/3 connection failed: {error:#}");
                }
            });
        }
    });

    Ok(Some(crate::Http3ListenerHandle {
        local_addr,
        join_handle,
    }))
}

fn build_quinn_server_config(
    config: &tokio_rustls::rustls::ServerConfig,
) -> Result<quinn::ServerConfig> {
    let mut tls_config = config.clone();
    tls_config.max_early_data_size = u32::MAX;
    tls_config.alpn_protocols = vec![b"h3".to_vec()];
    Ok(quinn::ServerConfig::with_crypto(Arc::new(
        QuicServerConfig::try_from(tls_config)?,
    )))
}

async fn handle_http3_connection(app: Router, connection: quinn::Connection) -> Result<()> {
    let mut incoming = h3::server::builder()
        .build(h3_quinn::Connection::new(connection))
        .await
        .context("failed to initialize HTTP/3 connection")?;

    loop {
        match incoming.accept().await {
            Ok(Some(resolver)) => {
                let app = app.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_http3_request(app, resolver).await {
                        tracing::warn!("HTTP/3 request handling failed: {error:#}");
                    }
                });
            }
            Ok(None) => return Ok(()),
            Err(error) => return Err(anyhow!("HTTP/3 accept loop failed: {error}")),
        }
    }
}

async fn handle_http3_request(
    app: Router,
    resolver: RequestResolver<h3_quinn::Connection, Bytes>,
) -> Result<()> {
    let (request, mut stream) = resolver
        .resolve_request()
        .await
        .context("failed to decode HTTP/3 request")?;
    let (parts, _) = request.into_parts();
    let mut body = BytesMut::new();
    while let Some(chunk) = stream
        .recv_data()
        .await
        .context("failed to receive HTTP/3 request body chunk")?
    {
        let mut chunk = chunk;
        let bytes = chunk.copy_to_bytes(chunk.remaining());
        body.extend_from_slice(&bytes);
    }
    let request_trailers = stream
        .recv_trailers()
        .await
        .context("failed to receive HTTP/3 request trailers")?
        .unwrap_or_else(HeaderMap::new);

    let mut request = Request::from_parts(parts, Body::from(body.freeze()));
    request.extensions_mut().insert(request_trailers);
    let response = app
        .oneshot(request)
        .await
        .map_err(|error| anyhow!("HTTP/3 router dispatch failed: {error}"))?;
    send_http3_response(stream, response).await
}

async fn send_http3_response(
    mut stream: h3::server::RequestStream<
        <h3_quinn::Connection as h3::quic::OpenStreams<Bytes>>::BidiStream,
        Bytes,
    >,
    response: Response,
) -> Result<()> {
    let (parts, body) = response.into_parts();
    let collected = body
        .collect()
        .await
        .context("failed to collect HTTP/3 response body")?;
    let response_trailers = collected.trailers().cloned();
    let body_bytes = collected.to_bytes();

    let mut response_builder = Response::builder().status(parts.status);
    for (name, value) in &parts.headers {
        response_builder = response_builder.header(name, value);
    }
    let response_head = response_builder
        .body(())
        .context("failed to build HTTP/3 response head")?;
    stream
        .send_response(response_head)
        .await
        .context("failed to send HTTP/3 response head")?;
    if !body_bytes.is_empty() {
        stream
            .send_data(body_bytes)
            .await
            .context("failed to send HTTP/3 response body")?;
    }
    if let Some(trailers) = response_trailers {
        stream
            .send_trailers(trailers)
            .await
            .context("failed to send HTTP/3 response trailers")?;
    }
    stream
        .finish()
        .await
        .context("failed to finish HTTP/3 stream")
}
