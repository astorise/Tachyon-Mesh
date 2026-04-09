use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    http::{HeaderMap, HeaderValue, StatusCode},
};
use rustls_pemfile::{certs, private_key};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::BufReader,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Mutex as TokioMutex;
use tokio_rustls::rustls::{self, ServerConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CertificateMaterial {
    pub(crate) domain: String,
    pub(crate) certificate_pem: String,
    pub(crate) private_key_pem: String,
}

#[derive(Default)]
pub(crate) struct TlsManager {
    cache: Mutex<HashMap<String, Arc<ServerConfig>>>,
    inflight: TokioMutex<HashMap<String, Arc<TokioMutex<()>>>>,
    provisions: AtomicUsize,
}

impl TlsManager {
    pub(crate) async fn server_config_for_domain(
        &self,
        state: &crate::AppState,
        domain: &str,
    ) -> Result<Arc<ServerConfig>> {
        let normalized = normalize_domain(domain)?;
        {
            let cache = self
                .cache
                .lock()
                .map_err(|_| anyhow!("TLS certificate cache lock poisoned"))?;
            if let Some(config) = cache.get(&normalized) {
                return Ok(Arc::clone(config));
            }
        }

        ensure_known_domain(state, &normalized)?;
        let domain_lock = self.domain_lock(&normalized).await;
        let _guard = domain_lock.lock().await;

        {
            let cache = self
                .cache
                .lock()
                .map_err(|_| anyhow!("TLS certificate cache lock poisoned"))?;
            if let Some(config) = cache.get(&normalized) {
                return Ok(Arc::clone(config));
            }
        }

        let material = if let Some(material) = self.load_persisted_material(state, &normalized)? {
            material
        } else {
            let material = self.provision_via_cert_manager(state, &normalized).await?;
            self.provisions.fetch_add(1, Ordering::Relaxed);
            material
        };

        let config = Arc::new(build_server_config(&material)?);
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow!("TLS certificate cache lock poisoned"))?;
        cache.insert(normalized, Arc::clone(&config));
        Ok(config)
    }

    #[cfg(test)]
    pub(crate) fn provision_count(&self) -> usize {
        self.provisions.load(Ordering::Relaxed)
    }

    async fn domain_lock(&self, domain: &str) -> Arc<TokioMutex<()>> {
        let mut locks = self.inflight.lock().await;
        Arc::clone(
            locks
                .entry(domain.to_owned())
                .or_insert_with(|| Arc::new(TokioMutex::new(()))),
        )
    }

    fn load_persisted_material(
        &self,
        state: &crate::AppState,
        domain: &str,
    ) -> Result<Option<CertificateMaterial>> {
        let runtime = state.runtime.load_full();
        let Some(route) = runtime
            .config
            .sealed_route(crate::SYSTEM_CERT_MANAGER_ROUTE)
        else {
            return Ok(None);
        };
        let storage_path = storage_path_for_domain(domain);
        let resolved = crate::resolve_storage_write_target(route, &storage_path)
            .map_err(|error| anyhow!("failed to resolve cert-manager storage path: {error}"))?;
        if !resolved.host_target.exists() {
            return Ok(None);
        }

        let payload = std::fs::read_to_string(&resolved.host_target).with_context(|| {
            format!(
                "failed to read persisted certificate bundle `{}`",
                resolved.host_target.display()
            )
        })?;

        serde_json::from_str(&payload)
            .with_context(|| {
                format!(
                    "failed to parse persisted certificate bundle `{}`",
                    resolved.host_target.display()
                )
            })
            .map(Some)
    }

    async fn provision_via_cert_manager(
        &self,
        state: &crate::AppState,
        domain: &str,
    ) -> Result<CertificateMaterial> {
        let runtime = state.runtime.load_full();
        let route = runtime
            .config
            .sealed_route(crate::SYSTEM_CERT_MANAGER_ROUTE)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "TLS domain `{domain}` requires the `{}` system route to be sealed",
                    crate::SYSTEM_CERT_MANAGER_ROUTE
                )
            })?;
        let function_name = crate::select_route_module(&route, &HeaderMap::new())
            .map_err(|error| anyhow!("failed to resolve cert-manager target module: {error}"))?;
        let uri = format!(
            "{}?domain={domain}&storage_path={}&mode={}",
            crate::SYSTEM_CERT_MANAGER_ROUTE,
            storage_path_for_domain(domain),
            crate::ACME_STAGING_MOCK_MODE
        );
        let identity_token = state
            .host_identity
            .sign_route(&route)
            .context("failed to sign cert-manager caller identity")?;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(
            crate::TACHYON_IDENTITY_HEADER,
            HeaderValue::from_str(&format!("Bearer {identity_token}"))
                .context("failed to encode cert-manager caller identity header")?,
        );
        let engine = runtime.engine.clone();
        let config = runtime.config.clone();
        let runtime_telemetry = state.telemetry.clone();
        let secret_access = crate::SecretAccess::from_route(&route, &state.secrets_vault);
        let host_identity = Arc::clone(&state.host_identity);
        let storage_broker = Arc::clone(&state.storage_broker);
        let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
        #[cfg(feature = "ai-inference")]
        let ai_runtime = Arc::clone(&runtime.ai_runtime);

        let response = tokio::task::spawn_blocking(move || {
            crate::execute_guest(
                &engine,
                &function_name,
                crate::GuestRequest::new("POST", uri, Bytes::new()),
                &route,
                crate::GuestExecutionContext {
                    config,
                    sampled_execution: false,
                    runtime_telemetry,
                    secret_access,
                    request_headers,
                    host_identity,
                    storage_broker,
                    telemetry: None,
                    concurrency_limits,
                    propagated_headers: Vec::new(),
                    #[cfg(feature = "ai-inference")]
                    ai_runtime,
                },
            )
        })
        .await
        .context("cert-manager execution task failed")??;

        match response.output {
            crate::GuestExecutionOutput::Http(http) if http.status == StatusCode::OK => {
                serde_json::from_slice(&http.body)
                    .context("failed to decode certificate material returned by cert-manager")
            }
            crate::GuestExecutionOutput::Http(http) => Err(anyhow!(
                "cert-manager returned HTTP {}: {}",
                http.status,
                String::from_utf8_lossy(&http.body)
            )),
            other => Err(anyhow!(
                "cert-manager returned an unexpected guest output variant: {other:?}"
            )),
        }
    }
}

fn build_server_config(material: &CertificateMaterial) -> Result<ServerConfig> {
    let mut cert_reader = BufReader::new(material.certificate_pem.as_bytes());
    let cert_chain = certs(&mut cert_reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to parse certificate PEM")?;
    let mut key_reader = BufReader::new(material.private_key_pem.as_bytes());
    let private_key = private_key(&mut key_reader)
        .context("failed to parse private key PEM")?
        .ok_or_else(|| anyhow!("certificate bundle did not contain a private key"))?;

    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)
        .context("failed to construct rustls server config")?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

fn ensure_known_domain(state: &crate::AppState, domain: &str) -> Result<()> {
    let runtime = state.runtime.load_full();
    if runtime.config.route_for_domain(domain).is_some() {
        Ok(())
    } else {
        Err(anyhow!("domain `{domain}` is not sealed for native TLS"))
    }
}

pub(crate) fn normalize_domain(value: &str) -> Result<String> {
    let trimmed = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(anyhow!("domain must not be empty"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains(' ') {
        return Err(anyhow!("domain must be a hostname without path separators"));
    }
    Ok(trimmed)
}

pub(crate) fn storage_path_for_domain(domain: &str) -> String {
    format!(
        "{}/{}.json",
        crate::CERT_MANAGER_GUEST_CERT_DIR,
        domain.replace('*', "_")
    )
}
