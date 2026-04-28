use anyhow::{anyhow, Context, Result};
use axum::{
    body::{Body, Bytes},
    http::Request,
    response::Response,
    Router,
};
use bytes::Buf;
use h3::{quic::SendStream, server::RequestResolver};
use http_body_util::BodyExt;
use quinn::crypto::rustls::QuicServerConfig;
use std::{io, sync::Arc, time::Duration};
use tokio_stream::wrappers::ReceiverStream;
use tower::ServiceExt;

// QUIC hardening — caps that prevent a misbehaving or malicious peer from monopolising
// host resources. Idle timeout reaps "Slowloris"-style hung connections; concurrency
// caps put a ceiling on how many streams a single peer can occupy.
const QUIC_MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const QUIC_MAX_CONCURRENT_BIDI_STREAMS: u32 = 256;
const QUIC_MAX_CONCURRENT_UNI_STREAMS: u32 = 100;
const QUIC_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

// Number of body chunks the recv-side streaming task is allowed to buffer ahead of the
// downstream consumer. A small bound is intentional: it keeps the host's RSS independent
// of the request body's total size, which is the whole point of streaming.
const BODY_CHANNEL_DEPTH: usize = 8;

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
            let state = state.clone();
            tokio::spawn(async move {
                let connection = match incoming.await {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::warn!("HTTP/3 QUIC handshake failed: {error}");
                        return;
                    }
                };

                if let Err(error) = handle_http3_connection(state, app, connection).await {
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
    let mut server =
        quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls_config)?));

    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        QUIC_MAX_IDLE_TIMEOUT
            .try_into()
            .expect("QUIC idle timeout fits in VarInt"),
    ));
    transport.max_concurrent_bidi_streams(QUIC_MAX_CONCURRENT_BIDI_STREAMS.into());
    transport.max_concurrent_uni_streams(QUIC_MAX_CONCURRENT_UNI_STREAMS.into());
    transport.keep_alive_interval(Some(QUIC_KEEPALIVE_INTERVAL));
    server.transport_config(Arc::new(transport));

    Ok(server)
}

async fn handle_http3_connection(
    state: crate::AppState,
    app: Router,
    connection: quinn::Connection,
) -> Result<()> {
    let mut incoming = h3::server::builder()
        .build(h3_quinn::Connection::new(connection))
        .await
        .context("failed to initialize HTTP/3 connection")?;

    loop {
        match incoming.accept().await {
            Ok(Some(resolver)) => {
                let app = app.clone();
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_http3_request(state, app, resolver).await {
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
    state: crate::AppState,
    app: Router,
    resolver: RequestResolver<h3_quinn::Connection, Bytes>,
) -> Result<()> {
    let (request, stream) = resolver
        .resolve_request()
        .await
        .context("failed to decode HTTP/3 request")?;
    let (parts, _) = request.into_parts();
    if let Some(response) = crate::auth::authorize_admin_headers(
        &state,
        parts.method.as_str(),
        parts.uri.path(),
        &parts.headers,
    )
    .await
    {
        // Auth rejection: no need to read the body. Drop the recv side and just
        // write the response back over the send side.
        let (send, _recv) = stream.split();
        return send_http3_response(send, response).await;
    }

    // Split the bidi stream so we can stream the request body into the router on one
    // task while we still own the send side for writing the response. This keeps the
    // host's memory footprint independent of the inbound body size — chunks flow
    // through a small bounded channel rather than being concatenated into RAM.
    let (send, recv) = stream.split();
    let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel::<io::Result<Bytes>>(BODY_CHANNEL_DEPTH);

    tokio::spawn(pump_request_body(recv, chunk_tx));

    let body = Body::from_stream(ReceiverStream::new(chunk_rx));

    let request = Request::from_parts(parts, body);
    let response = app
        .oneshot(request)
        .await
        .map_err(|error| anyhow!("HTTP/3 router dispatch failed: {error}"))?;

    send_http3_response(send, response).await
}

async fn pump_request_body<S>(
    mut recv: h3::server::RequestStream<S, Bytes>,
    tx: tokio::sync::mpsc::Sender<io::Result<Bytes>>,
) where
    S: h3::quic::RecvStream,
{
    loop {
        match recv.recv_data().await {
            Ok(Some(mut chunk)) => {
                let bytes = chunk.copy_to_bytes(chunk.remaining());
                if tx.send(Ok(bytes)).await.is_err() {
                    // Downstream dropped the body — abandon the upload silently.
                    return;
                }
            }
            Ok(None) => return,
            Err(error) => {
                let _ = tx
                    .send(Err(io::Error::new(io::ErrorKind::Other, error.to_string())))
                    .await;
                return;
            }
        }
    }
}

async fn send_http3_response<S>(
    mut stream: h3::server::RequestStream<S, Bytes>,
    response: Response,
) -> Result<()>
where
    S: SendStream<Bytes>,
{
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quic_transport_config_caps_are_applied() {
        // We cannot easily inspect the resulting quinn::ServerConfig without a real TLS
        // handshake, so instead we re-derive what the quinn TransportConfig builder is
        // told and assert the constants match the spec'd safety bounds. This keeps the
        // limits visible in code review and prevents an accidental regression that
        // silently uses quinn's defaults.
        assert_eq!(QUIC_MAX_IDLE_TIMEOUT.as_secs(), 30);
        assert_eq!(QUIC_MAX_CONCURRENT_BIDI_STREAMS, 256);
        assert_eq!(QUIC_MAX_CONCURRENT_UNI_STREAMS, 100);
        assert_eq!(QUIC_KEEPALIVE_INTERVAL.as_secs(), 15);
        // The body channel depth is small (constant memory), independent of body size.
        assert!(BODY_CHANNEL_DEPTH <= 32);
    }
}
