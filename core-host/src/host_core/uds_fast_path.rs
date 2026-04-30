use super::*;

#[cfg(unix)]
impl UdsFastPathRegistry {
    #[cfg(test)]
    pub(crate) fn with_discovery_dir(path: PathBuf) -> Self {
        let registry = Self::default();
        *registry
            .discovery_dir_override
            .lock()
            .expect("UDS discovery override should not be poisoned") = Some(path);
        registry
    }

    pub(crate) fn discovery_dir(&self) -> PathBuf {
        if let Some(path) = self
            .discovery_dir_override
            .lock()
            .expect("UDS discovery override should not be poisoned")
            .clone()
        {
            return path;
        }

        std::env::var_os(TACHYON_DISCOVERY_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DISCOVERY_DIR))
    }

    pub(crate) fn bind_local_listener(&self, config: &IntegrityConfig) -> Result<UnixListener> {
        let discovery_dir = self.discovery_dir();
        fs::create_dir_all(&discovery_dir).with_context(|| {
            format!(
                "failed to create UDS discovery directory `{}`",
                discovery_dir.display()
            )
        })?;

        let host_id = Uuid::new_v4().simple().to_string();
        let file_stem = format!("h-{}", &host_id[..12]);
        let socket_path = discovery_dir.join(format!("{file_stem}.sock"));
        let metadata_path = discovery_dir.join(format!("{file_stem}.json"));
        if socket_path.exists() {
            remove_path_if_exists(&socket_path)?;
        }

        let listener = UnixListener::bind(&socket_path).with_context(|| {
            format!(
                "failed to bind UDS fast-path listener at `{}`",
                socket_path.display()
            )
        })?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o660)).with_context(
            || {
                format!(
                    "failed to tighten permissions on UDS socket `{}`",
                    socket_path.display()
                )
            },
        )?;

        let metadata = UdsPeerMetadata {
            host_id,
            ip: discovery_publish_ip(config)?,
            socket_path: socket_path.display().to_string(),
            protocols: vec!["http/1.1".to_owned(), "h2".to_owned()],
            pressure_state: PeerPressureState::Idle,
            last_pressure_update_unix_ms: 0,
        };
        fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata)
                .context("failed to serialize UDS peer metadata")?,
        )
        .with_context(|| {
            format!(
                "failed to publish UDS peer metadata `{}`",
                metadata_path.display()
            )
        })?;

        let peer = DiscoveredUdsPeer {
            metadata_path: metadata_path.clone(),
            socket_path: socket_path.clone(),
            metadata: metadata.clone(),
        };
        self.peers
            .lock()
            .expect("UDS peer cache should not be poisoned")
            .insert(metadata.host_id.clone(), peer);
        *self
            .local_endpoint
            .lock()
            .expect("local UDS endpoint should not be poisoned") = Some(LocalUdsEndpoint {
            metadata_path,
            socket_path,
        });

        Ok(listener)
    }

    pub(crate) fn discover_peer_for_url(&self, url: &str) -> Option<DiscoveredUdsPeer> {
        let host = reqwest::Url::parse(url).ok()?.host_str()?.to_owned();
        let now_unix_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let mut candidates = self
            .refresh_peers()
            .into_values()
            .filter(|peer| peer.metadata.ip == host)
            .filter(|peer| {
                peer.metadata.last_pressure_update_unix_ms == 0
                    || now_unix_ms.saturating_sub(peer.metadata.last_pressure_update_unix_ms)
                        <= PEER_PRESSURE_STALE_AFTER.as_millis() as u64
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }
        if candidates.len() == 1 {
            return candidates.pop();
        }

        let mut rng = rand::rng();
        let first_index = rng.random_range(0..candidates.len());
        let mut second_index = rng.random_range(0..candidates.len() - 1);
        if second_index >= first_index {
            second_index += 1;
        }
        let first = &candidates[first_index];
        let second = &candidates[second_index];
        let preferred = if first.metadata.pressure_state <= second.metadata.pressure_state {
            first
        } else {
            second
        };
        Some(preferred.clone())
    }

    pub(crate) fn active_peer_count(&self) -> usize {
        let peers = self.refresh_peers();
        let local_host = self
            .local_endpoint
            .lock()
            .expect("local UDS endpoint should not be poisoned")
            .as_ref()
            .and_then(|endpoint| {
                fs::read(&endpoint.metadata_path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<UdsPeerMetadata>(&bytes).ok())
                    .map(|metadata| metadata.host_id)
            });

        peers
            .values()
            .filter(|peer| Some(peer.metadata.host_id.clone()) != local_host)
            .count()
    }

    pub(crate) fn write_local_pressure_state(
        &self,
        pressure_state: PeerPressureState,
        updated_at_unix_ms: u64,
    ) -> Result<()> {
        let Some(endpoint) = self
            .local_endpoint
            .lock()
            .expect("local UDS endpoint should not be poisoned")
            .clone()
        else {
            return Ok(());
        };
        let mut metadata: UdsPeerMetadata =
            serde_json::from_slice(&fs::read(&endpoint.metadata_path).with_context(|| {
                format!(
                    "failed to read local UDS metadata `{}`",
                    endpoint.metadata_path.display()
                )
            })?)
            .context("failed to parse local UDS metadata")?;
        metadata.pressure_state = pressure_state;
        metadata.last_pressure_update_unix_ms = updated_at_unix_ms;
        fs::write(
            &endpoint.metadata_path,
            serde_json::to_vec_pretty(&metadata).context("failed to serialize pressure state")?,
        )
        .with_context(|| {
            format!(
                "failed to persist local pressure state to `{}`",
                endpoint.metadata_path.display()
            )
        })?;
        self.peers
            .lock()
            .expect("UDS peer cache should not be poisoned")
            .insert(
                metadata.host_id.clone(),
                DiscoveredUdsPeer {
                    metadata_path: endpoint.metadata_path,
                    socket_path: endpoint.socket_path,
                    metadata,
                },
            );
        Ok(())
    }

    pub(crate) fn note_connect_failure(&self, peer: &DiscoveredUdsPeer) {
        self.peers
            .lock()
            .expect("UDS peer cache should not be poisoned")
            .remove(&peer.metadata.host_id);
        if !peer.socket_path.exists() {
            let _ = fs::remove_file(&peer.metadata_path);
        }
    }

    pub(crate) fn refresh_peers(&self) -> HashMap<String, DiscoveredUdsPeer> {
        let discovery_dir = self.discovery_dir();
        let mut discovered = HashMap::new();
        let entries = match fs::read_dir(&discovery_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.peers
                    .lock()
                    .expect("UDS peer cache should not be poisoned")
                    .clear();
                return discovered;
            }
            Err(_) => {
                return self
                    .peers
                    .lock()
                    .expect("UDS peer cache should not be poisoned")
                    .clone()
            }
        };

        for entry in entries.flatten() {
            let metadata_path = entry.path();
            if metadata_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let metadata = match fs::read(&metadata_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<UdsPeerMetadata>(&bytes).ok())
            {
                Some(metadata) => metadata,
                None => continue,
            };

            let socket_path = PathBuf::from(&metadata.socket_path);
            if !socket_path.exists() {
                let _ = fs::remove_file(&metadata_path);
                continue;
            }

            discovered.insert(
                metadata.host_id.clone(),
                DiscoveredUdsPeer {
                    metadata_path,
                    socket_path,
                    metadata,
                },
            );
        }

        *self
            .peers
            .lock()
            .expect("UDS peer cache should not be poisoned") = discovered.clone();
        discovered
    }
}

#[cfg(unix)]
impl Drop for UdsFastPathRegistry {
    fn drop(&mut self) {
        if Arc::strong_count(&self.local_endpoint) != 1 {
            return;
        }

        let local_endpoint = self
            .local_endpoint
            .lock()
            .expect("local UDS endpoint should not be poisoned")
            .clone();
        if let Some(endpoint) = local_endpoint {
            let _ = fs::remove_file(endpoint.metadata_path);
            let _ = fs::remove_file(endpoint.socket_path);
        }
    }
}

#[cfg(not(unix))]
impl UdsFastPathRegistry {
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn with_discovery_dir(_path: PathBuf) -> Self {
        Self
    }

    pub(crate) fn active_peer_count(&self) -> usize {
        0
    }

    pub(crate) fn write_local_pressure_state(
        &self,
        _pressure_state: PeerPressureState,
        _updated_at_unix_ms: u64,
    ) -> Result<()> {
        Ok(())
    }
}
