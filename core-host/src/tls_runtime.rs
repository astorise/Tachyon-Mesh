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
    net::SocketAddr,
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

#[derive(Clone)]
pub(crate) struct MtlsGatewayConfig {
    pub(crate) bind_address: SocketAddr,
    pub(crate) server_config: Arc<ServerConfig>,
}

#[derive(Default)]
pub(crate) struct TlsManager {
    cache: Mutex<HashMap<String, Arc<ServerConfig>>>,
    inflight: TokioMutex<HashMap<String, Arc<TokioMutex<()>>>>,
    provisions: AtomicUsize,
}

impl TlsManager {
    pub(crate) async fn prime_from_store(&self, state: &crate::AppState) -> Result<()> {
        let domains = state
            .runtime
            .load_full()
            .config
            .routes
            .iter()
            .flat_map(|route| route.domains.iter().cloned())
            .collect::<Vec<_>>();
        let core_store = Arc::clone(&state.core_store);

        let persisted = tokio::task::spawn_blocking(move || {
            let mut materials = Vec::new();
            for domain in domains {
                let Some(payload) =
                    core_store.get(crate::store::CoreStoreBucket::TlsCerts, &domain)?
                else {
                    continue;
                };
                let material: CertificateMaterial =
                    serde_json::from_slice(&payload).with_context(|| {
                        format!("failed to decode cached TLS bundle for `{domain}`")
                    })?;
                materials.push((domain, material));
            }
            Ok::<_, anyhow::Error>(materials)
        })
        .await
        .context("TLS cache priming task failed")??;

        if persisted.is_empty() {
            return Ok(());
        }

        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow!("TLS certificate cache lock poisoned"))?;
        for (domain, material) in persisted {
            cache.insert(domain, Arc::new(build_server_config(&material)?));
        }
        Ok(())
    }

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

        let (material, from_store) = if let Some((material, from_store)) =
            self.load_persisted_material(state, &normalized).await?
        {
            (material, from_store)
        } else {
            let material = self.provision_via_cert_manager(state, &normalized).await?;
            self.provisions.fetch_add(1, Ordering::Relaxed);
            (material, false)
        };

        if !from_store {
            self.persist_material(state, &material).await?;
        }

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

    async fn load_persisted_material(
        &self,
        state: &crate::AppState,
        domain: &str,
    ) -> Result<Option<(CertificateMaterial, bool)>> {
        let domain_key = domain.to_owned();
        let core_store = Arc::clone(&state.core_store);
        if let Some(payload) = tokio::task::spawn_blocking(move || {
            core_store.get(crate::store::CoreStoreBucket::TlsCerts, &domain_key)
        })
        .await
        .context("TLS core-store lookup task failed")??
        {
            let material = serde_json::from_slice(&payload)
                .with_context(|| format!("failed to parse cached TLS bundle for `{domain}`"))?;
            return Ok(Some((material, true)));
        }

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

        let material = serde_json::from_str(&payload).with_context(|| {
            format!(
                "failed to parse persisted certificate bundle `{}`",
                resolved.host_target.display()
            )
        })?;

        Ok(Some((material, false)))
    }

    async fn persist_material(
        &self,
        state: &crate::AppState,
        material: &CertificateMaterial,
    ) -> Result<()> {
        let domain = material.domain.clone();
        let payload = serde_json::to_vec(material)
            .context("failed to serialize TLS certificate material for persistence")?;
        let core_store = Arc::clone(&state.core_store);
        tokio::task::spawn_blocking(move || {
            core_store.put(crate::store::CoreStoreBucket::TlsCerts, &domain, &payload)
        })
        .await
        .context("TLS persistence task failed")?
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
        let bridge_manager = Arc::clone(&state.bridge_manager);
        let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
        let route_overrides = Arc::clone(&state.route_overrides);
        let host_load = Arc::clone(&state.host_load);
        let async_log_sender = state.async_log_sender.clone();
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
                    async_log_sender,
                    secret_access,
                    request_headers,
                    host_identity,
                    storage_broker,
                    bridge_manager,
                    telemetry: None,
                    concurrency_limits,
                    propagated_headers: Vec::new(),
                    route_overrides,
                    host_load,
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

pub(crate) fn load_mtls_gateway_config_from_env() -> Result<Option<MtlsGatewayConfig>> {
    let Some(server_certificate_pem) = std::env::var("TACHYON_MTLS_SERVER_CERT_PEM")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let server_key_pem = required_env("TACHYON_MTLS_SERVER_KEY_PEM")?;
    let ca_certificate_pem = required_env("TACHYON_MTLS_CA_CERT_PEM")?;
    let bind_address = std::env::var(crate::TACHYON_MTLS_ADDRESS_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "0.0.0.0:8443".to_owned())
        .parse()
        .context("failed to parse TACHYON_MTLS_ADDRESS")?;

    Ok(Some(MtlsGatewayConfig {
        bind_address,
        server_config: Arc::new(build_mtls_server_config(
            &server_certificate_pem,
            &server_key_pem,
            &ca_certificate_pem,
        )?),
    }))
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

pub(crate) fn build_mtls_server_config(
    certificate_pem: &str,
    private_key_pem: &str,
    ca_certificate_pem: &str,
) -> Result<ServerConfig> {
    let cert_chain = parse_cert_chain(certificate_pem)?;
    let private_key = parse_private_key(private_key_pem)?;
    let mut ca_reader = BufReader::new(ca_certificate_pem.as_bytes());
    let ca_certs = certs(&mut ca_reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to parse client CA PEM")?;
    let mut roots = rustls::RootCertStore::empty();
    let (added, _) = roots.add_parsable_certificates(ca_certs);
    if added == 0 {
        return Err(anyhow!(
            "client CA PEM did not contain any valid certificates"
        ));
    }
    let client_verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .context("failed to construct rustls client verifier")?;
    let mut config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(cert_chain, private_key)
        .context("failed to construct rustls mTLS server config")?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

fn parse_cert_chain(
    certificate_pem: &str,
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let mut cert_reader = BufReader::new(certificate_pem.as_bytes());
    certs(&mut cert_reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to parse certificate PEM")
}

fn parse_private_key(private_key_pem: &str) -> Result<rustls::pki_types::PrivateKeyDer<'static>> {
    let mut key_reader = BufReader::new(private_key_pem.as_bytes());
    private_key(&mut key_reader)
        .context("failed to parse private key PEM")?
        .ok_or_else(|| anyhow!("certificate bundle did not contain a private key"))
}

fn required_env(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing required environment variable `{name}`"))
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
