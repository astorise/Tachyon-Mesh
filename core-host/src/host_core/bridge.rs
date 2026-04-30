use super::*;

#[derive(Clone, Default)]
pub(crate) struct BridgeManager {
    pub(crate) inner: Arc<BridgeManagerInner>,
}

#[derive(Default)]
pub(crate) struct BridgeManagerInner {
    pub(crate) sessions: Mutex<HashMap<String, BridgeSession>>,
    pub(crate) active_relays: AtomicUsize,
    pub(crate) relayed_bytes: AtomicU64,
    pub(crate) telemetry: Mutex<BridgeTelemetryState>,
}

pub(crate) struct BridgeSession {
    pub(crate) abort_handle: tokio::task::AbortHandle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BridgeConfig {
    pub(crate) client_a_addr: String,
    pub(crate) client_b_addr: String,
    pub(crate) timeout_seconds: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BridgeAllocation {
    pub(crate) bridge_id: String,
    #[serde(default)]
    pub(crate) ip: String,
    pub(crate) port_a: u16,
    pub(crate) port_b: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BridgeTelemetrySnapshot {
    pub(crate) active_relays: u32,
    pub(crate) throughput_bytes_per_sec: u64,
    pub(crate) load_score: u8,
}

pub(crate) struct BridgeTelemetryState {
    pub(crate) last_total_bytes: u64,
    pub(crate) last_sampled_at: Instant,
    pub(crate) throughput_bytes_per_sec: u64,
}

impl Default for BridgeTelemetryState {
    fn default() -> Self {
        Self {
            last_total_bytes: 0,
            last_sampled_at: Instant::now(),
            throughput_bytes_per_sec: 0,
        }
    }
}

impl BridgeManager {
    pub(crate) fn create_relay(
        &self,
        config: BridgeConfig,
    ) -> std::result::Result<BridgeAllocation, String> {
        let endpoint_a = parse_bridge_endpoint(&config.client_a_addr, "client_a_addr")?;
        let endpoint_b = parse_bridge_endpoint(&config.client_b_addr, "client_b_addr")?;
        let inactivity_timeout = Duration::from_secs(u64::from(config.timeout_seconds.max(1)));

        let socket_a = bind_bridge_socket()?;
        let socket_b = bind_bridge_socket()?;
        let port_a = socket_a
            .local_addr()
            .map_err(|error| format!("failed to resolve bridge port A: {error}"))?
            .port();
        let port_b = socket_b
            .local_addr()
            .map_err(|error| format!("failed to resolve bridge port B: {error}"))?
            .port();

        let bridge_id = Uuid::new_v4().to_string();
        let inner = Arc::clone(&self.inner);
        let cleanup_id = bridge_id.clone();
        let join_handle = tokio::spawn(async move {
            if let Err(error) = relay_bridge(
                socket_a,
                socket_b,
                endpoint_a,
                endpoint_b,
                inactivity_timeout,
                &inner,
            )
            .await
            {
                tracing::warn!(bridge_id = %cleanup_id, "dynamic bridge relay exited: {error}");
            }
            release_bridge_session(&inner, &cleanup_id);
        });

        let mut sessions = self
            .inner
            .sessions
            .lock()
            .expect("bridge session registry should not be poisoned");
        sessions.insert(
            bridge_id.clone(),
            BridgeSession {
                abort_handle: join_handle.abort_handle(),
            },
        );
        drop(sessions);
        self.inner.active_relays.fetch_add(1, Ordering::SeqCst);

        Ok(BridgeAllocation {
            bridge_id,
            ip: String::new(),
            port_a,
            port_b,
        })
    }

    pub(crate) fn destroy_relay(&self, bridge_id: &str) -> std::result::Result<(), String> {
        let session = self
            .inner
            .sessions
            .lock()
            .expect("bridge session registry should not be poisoned")
            .remove(bridge_id);
        let Some(session) = session else {
            return Err(format!("bridge `{bridge_id}` is not active"));
        };
        session.abort_handle.abort();
        self.inner.active_relays.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn active_relay_count(&self) -> usize {
        self.inner.active_relays.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn total_relayed_bytes(&self) -> u64 {
        self.inner.relayed_bytes.load(Ordering::SeqCst)
    }

    pub(crate) fn telemetry_snapshot(&self) -> BridgeTelemetrySnapshot {
        let total_bytes = self.inner.relayed_bytes.load(Ordering::SeqCst);
        let active_relays = self.inner.active_relays.load(Ordering::SeqCst) as u32;
        let mut telemetry = self
            .inner
            .telemetry
            .lock()
            .expect("bridge telemetry state should not be poisoned");
        let elapsed = telemetry.last_sampled_at.elapsed();
        if elapsed >= Duration::from_millis(250) {
            let delta = total_bytes.saturating_sub(telemetry.last_total_bytes);
            telemetry.throughput_bytes_per_sec = if elapsed.is_zero() {
                0
            } else {
                ((delta as u128 * 1_000_000_000_u128) / elapsed.as_nanos()) as u64
            };
            telemetry.last_total_bytes = total_bytes;
            telemetry.last_sampled_at = Instant::now();
        }

        BridgeTelemetrySnapshot {
            active_relays,
            throughput_bytes_per_sec: telemetry.throughput_bytes_per_sec,
            load_score: bridge_load_score(active_relays, telemetry.throughput_bytes_per_sec),
        }
    }
}

pub(crate) fn bridge_load_score(active_relays: u32, throughput_bytes_per_sec: u64) -> u8 {
    let relay_score = active_relays.saturating_mul(25).min(100) as u8;
    let throughput_score = ((throughput_bytes_per_sec / 50_000).min(100)) as u8;
    relay_score.max(throughput_score)
}

pub(crate) fn bind_bridge_socket() -> std::result::Result<tokio::net::UdpSocket, String> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("failed to bind dynamic bridge socket: {error}"))?;
    socket
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set dynamic bridge socket nonblocking: {error}"))?;
    tokio::net::UdpSocket::from_std(socket)
        .map_err(|error| format!("failed to convert dynamic bridge socket to tokio: {error}"))
}

pub(crate) fn parse_bridge_endpoint(
    value: &str,
    label: &str,
) -> std::result::Result<SocketAddr, String> {
    value
        .trim()
        .parse::<SocketAddr>()
        .map_err(|error| format!("failed to parse `{label}` as socket address: {error}"))
}

pub(crate) async fn relay_bridge(
    socket_a: tokio::net::UdpSocket,
    socket_b: tokio::net::UdpSocket,
    endpoint_a: SocketAddr,
    endpoint_b: SocketAddr,
    inactivity_timeout: Duration,
    inner: &BridgeManagerInner,
) -> std::result::Result<(), String> {
    let mut buffer_a = [0_u8; UDP_LAYER4_MAX_DATAGRAM_SIZE];
    let mut buffer_b = [0_u8; UDP_LAYER4_MAX_DATAGRAM_SIZE];
    let mut deadline = tokio::time::Instant::now() + inactivity_timeout;

    loop {
        let sleep = tokio::time::sleep_until(deadline);
        tokio::pin!(sleep);

        tokio::select! {
            _ = &mut sleep => return Ok(()),
            received = socket_a.recv_from(&mut buffer_a) => {
                let (size, _) = received.map_err(|error| format!("failed to receive bridge packet on port A: {error}"))?;
                socket_b
                    .send_to(&buffer_a[..size], endpoint_b)
                    .await
                    .map_err(|error| format!("failed to forward bridge packet to endpoint B: {error}"))?;
                inner.relayed_bytes.fetch_add(size as u64, Ordering::SeqCst);
                deadline = tokio::time::Instant::now() + inactivity_timeout;
            }
            received = socket_b.recv_from(&mut buffer_b) => {
                let (size, _) = received.map_err(|error| format!("failed to receive bridge packet on port B: {error}"))?;
                socket_a
                    .send_to(&buffer_b[..size], endpoint_a)
                    .await
                    .map_err(|error| format!("failed to forward bridge packet to endpoint A: {error}"))?;
                inner.relayed_bytes.fetch_add(size as u64, Ordering::SeqCst);
                deadline = tokio::time::Instant::now() + inactivity_timeout;
            }
        }
    }
}

pub(crate) fn release_bridge_session(inner: &BridgeManagerInner, bridge_id: &str) {
    let removed = inner
        .sessions
        .lock()
        .expect("bridge session registry should not be poisoned")
        .remove(bridge_id)
        .is_some();
    if removed {
        inner.active_relays.fetch_sub(1, Ordering::SeqCst);
    }
}
