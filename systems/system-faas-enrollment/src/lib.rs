use anyhow::{anyhow, Context, Result};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Debug)]
pub struct EnrollmentConfig {
    pub bootstrap_url: String,
    pub cert_output_path: PathBuf,
    pub poll_interval: Duration,
    pub max_polls: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartRequest {
    node_public_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartResponse {
    session_id: String,
    pin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentSession {
    pub session_id: String,
    pub pin: String,
    pub node_public_key_hex: String,
}

pub fn generate_node_keypair() -> SigningKey {
    SigningKey::from_bytes(&rand::random::<[u8; 32]>())
}

pub fn start_request_for_key(signing_key: &SigningKey) -> (String, EnrollmentSession) {
    let public_key = hex::encode(signing_key.verifying_key().to_bytes());
    (
        public_key.clone(),
        EnrollmentSession {
            session_id: String::new(),
            pin: String::new(),
            node_public_key_hex: public_key,
        },
    )
}

pub async fn run_enrollment(config: EnrollmentConfig) -> Result<PathBuf> {
    let signing_key = generate_node_keypair();
    let (node_public_key, _) = start_request_for_key(&signing_key);
    let client = reqwest::Client::new();
    let start_url = join_endpoint(&config.bootstrap_url, "/admin/enrollment/start");
    let start = client
        .post(&start_url)
        .json(&StartRequest { node_public_key })
        .send()
        .await
        .with_context(|| format!("failed to start enrollment via `{start_url}`"))?;
    if !start.status().is_success() {
        return Err(anyhow!(
            "enrollment start failed with {}: {}",
            start.status(),
            start.text().await.unwrap_or_default()
        ));
    }
    let start: StartResponse = start
        .json()
        .await
        .context("failed to decode enrollment start response")?;
    println!(
        "[ENROLLMENT] Waiting for approval. Enter PIN in Tachyon-UI: {}",
        start.pin
    );

    let poll_url = join_endpoint(
        &config.bootstrap_url,
        &format!("/admin/enrollment/poll/{}", start.session_id),
    );
    for _ in 0..config.max_polls {
        let response = client
            .get(&poll_url)
            .send()
            .await
            .with_context(|| format!("failed to poll enrollment via `{poll_url}`"))?;
        match response.status().as_u16() {
            200 => {
                let cert_hex = response
                    .text()
                    .await
                    .context("failed to read signed certificate response")?;
                let cert = hex::decode(cert_hex.trim())
                    .context("enrollment poll returned non-hex certificate")?;
                if let Some(parent) = config.cert_output_path.parent() {
                    tokio::fs::create_dir_all(parent).await.with_context(|| {
                        format!(
                            "failed to create enrollment cert dir `{}`",
                            parent.display()
                        )
                    })?;
                }
                tokio::fs::write(&config.cert_output_path, cert)
                    .await
                    .with_context(|| {
                        format!(
                            "failed to persist enrollment certificate `{}`",
                            config.cert_output_path.display()
                        )
                    })?;
                println!("[ENROLLMENT] ENROLLMENT_COMPLETE");
                return Ok(config.cert_output_path);
            }
            204 => tokio::time::sleep(config.poll_interval).await,
            410 => {
                return Err(anyhow!(
                    "enrollment rejected: {}",
                    response.text().await.unwrap_or_default()
                ));
            }
            status => {
                return Err(anyhow!(
                    "enrollment poll failed with {status}: {}",
                    response.text().await.unwrap_or_default()
                ));
            }
        }
    }

    Err(anyhow!("enrollment timed out before approval"))
}

fn join_endpoint(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_is_hex_public_key() {
        let signing_key = generate_node_keypair();
        let (public_key, session) = start_request_for_key(&signing_key);
        assert_eq!(public_key.len(), 64);
        assert_eq!(session.node_public_key_hex, public_key);
        assert!(hex::decode(public_key).is_ok());
    }

    #[test]
    fn endpoint_joining_is_stable() {
        assert_eq!(
            join_endpoint("https://node.example/", "/admin/enrollment/start"),
            "https://node.example/admin/enrollment/start"
        );
    }
}
