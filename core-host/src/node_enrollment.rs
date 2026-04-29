//! Host-side state for the device-flow node enrollment.
//!
//! When a fresh, unenrolled `core-host` boots, it generates a short, human-
//! readable PIN, opens an outbound channel to `IntegrityConfig::enrollment_endpoint`,
//! and waits for an admin to enter that PIN through Tachyon Studio while
//! connected to any active mesh node. The active node verifies the PIN, signs
//! the new node's CSR, and routes the certificate back through the open
//! channel.
//!
//! This module owns the **operator-side** half: the `EnrollmentManager` that
//! tracks pending sessions, generates / verifies PINs, and surfaces signed
//! credentials to the admin API. The unenrolled-node side (the outbound
//! long-poll client) is wired in Session C alongside `system-faas-mesh-overlay`,
//! once the Noise tunnel infrastructure is in place. Until then, the manager
//! still satisfies the spec scenarios that drive operator UX.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

/// PIN format: three uppercase chars + dash + three uppercase chars (e.g. "A7X-92B").
/// 33⁶ ≈ 1.3 G combinations — enough entropy that an admin can read the PIN
/// off the screen without it being cheaply brute-forceable in the operator
/// approval window. The pin alphabet excludes `O/0/I/1` to avoid mis-reads.
const PIN_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const PIN_HALF_LEN: usize = 3;

/// How long an unapproved session is retained before garbage collection. Long
/// enough that an admin can finish reading the PIN aloud over a phone call;
/// short enough that an abandoned session isn't a permanent attack surface.
const ENROLLMENT_SESSION_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnrollmentSession {
    pub session_id: String,
    pub pin: String,
    /// Hex-encoded ed25519 public key the unenrolled node submitted with its
    /// outbound enrollment request. The active node will sign this key with
    /// the cluster CA on operator approval.
    pub node_public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnrollmentOutcome {
    /// Operator approved. The carried bytes are the signed certificate the
    /// unenrolled node will use as its mTLS identity going forward.
    Approved { signed_certificate: Vec<u8> },
    /// Operator rejected (or the PIN didn't match).
    Rejected { reason: String },
}

#[derive(Default)]
pub(crate) struct EnrollmentManager {
    sessions: Mutex<HashMap<String, EnrollmentEntry>>,
}

struct EnrollmentEntry {
    session: EnrollmentSession,
    started_at: Instant,
    outcome: Option<EnrollmentOutcome>,
}

impl EnrollmentManager {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Begin a new enrollment session for a node that just submitted its public
    /// key. Returns the human-readable PIN the operator must enter into Tachyon
    /// Studio to approve the enrollment.
    pub(crate) fn start_session(&self, node_public_key: String) -> EnrollmentSession {
        let pin = generate_pin();
        let session_id = format!("enroll-{}", hex::encode(rand::random::<[u8; 16]>()));
        let session = EnrollmentSession {
            session_id: session_id.clone(),
            pin: pin.clone(),
            node_public_key,
        };
        let mut sessions = self
            .sessions
            .lock()
            .expect("enrollment session map should not be poisoned");
        self.gc_expired(&mut sessions);
        sessions.insert(
            session_id.clone(),
            EnrollmentEntry {
                session: session.clone(),
                started_at: Instant::now(),
                outcome: None,
            },
        );
        session
    }

    /// Operator approves a pending session by entering the PIN via Tachyon
    /// Studio. The active node validates the PIN against the recorded session
    /// and stages the signed certificate for the unenrolled node to fetch.
    /// Returns `Ok(())` on success, `Err(reason)` on PIN mismatch / unknown id.
    pub(crate) fn approve(
        &self,
        session_id: &str,
        pin: &str,
        signed_certificate: Vec<u8>,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .expect("enrollment session map should not be poisoned");
        self.gc_expired(&mut sessions);
        let entry = sessions
            .get_mut(session_id)
            .ok_or_else(|| "unknown enrollment session".to_owned())?;
        if entry.outcome.is_some() {
            return Err("enrollment session already finalized".to_owned());
        }
        if entry.session.pin != pin {
            return Err("PIN mismatch".to_owned());
        }
        entry.outcome = Some(EnrollmentOutcome::Approved { signed_certificate });
        Ok(())
    }

    /// Operator rejects a pending session, e.g. because the device asking to
    /// enroll wasn't expected. Surfaces a reason to the unenrolled node so the
    /// device-flow client can log + retry.
    #[allow(dead_code)]
    pub(crate) fn reject(&self, session_id: &str, reason: String) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .expect("enrollment session map should not be poisoned");
        let entry = sessions
            .get_mut(session_id)
            .ok_or_else(|| "unknown enrollment session".to_owned())?;
        if entry.outcome.is_some() {
            return Err("enrollment session already finalized".to_owned());
        }
        entry.outcome = Some(EnrollmentOutcome::Rejected { reason });
        Ok(())
    }

    /// Polled by the unenrolled node's outbound channel to learn whether the
    /// admin has approved (or rejected) the session. Returns `None` while the
    /// session is still pending. When the outcome is delivered, the entry is
    /// removed from the map — preventing replay.
    pub(crate) fn poll_outcome(&self, session_id: &str) -> Option<EnrollmentOutcome> {
        let mut sessions = self
            .sessions
            .lock()
            .expect("enrollment session map should not be poisoned");
        sessions.get(session_id)?.outcome.as_ref()?;
        let entry = sessions.remove(session_id)?;
        entry.outcome
    }

    fn gc_expired(&self, sessions: &mut HashMap<String, EnrollmentEntry>) {
        let now = Instant::now();
        sessions.retain(|_, entry| {
            entry.started_at.elapsed() < ENROLLMENT_SESSION_TTL || entry.outcome.is_some()
        });
        // Also drop *finalized* sessions that overshot the TTL — the unenrolled
        // node should have already polled them by then.
        sessions.retain(|_, entry| {
            entry.outcome.is_none() || now.duration_since(entry.started_at) < ENROLLMENT_SESSION_TTL
        });
    }

    #[cfg(test)]
    pub(crate) fn session_count(&self) -> usize {
        self.sessions
            .lock()
            .expect("enrollment session map should not be poisoned")
            .len()
    }
}

fn generate_pin() -> String {
    let alphabet = PIN_ALPHABET;
    let mut bytes = [0u8; PIN_HALF_LEN * 2];
    for slot in bytes.iter_mut() {
        let r: [u8; 1] = rand::random();
        *slot = alphabet[(r[0] as usize) % alphabet.len()];
    }
    let half_a = std::str::from_utf8(&bytes[..PIN_HALF_LEN]).expect("PIN alphabet is ASCII");
    let half_b = std::str::from_utf8(&bytes[PIN_HALF_LEN..]).expect("PIN alphabet is ASCII");
    format!("{half_a}-{half_b}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_have_the_documented_format() {
        let pin = generate_pin();
        assert_eq!(pin.len(), PIN_HALF_LEN * 2 + 1);
        assert!(pin.contains('-'));
        for c in pin.chars().filter(|c| *c != '-') {
            assert!(
                PIN_ALPHABET.contains(&(c as u8)),
                "PIN char `{c}` not in alphabet",
            );
        }
    }

    #[test]
    fn approve_then_poll_returns_signed_certificate_and_removes_session() {
        let mgr = EnrollmentManager::new();
        let session = mgr.start_session("nodepubkey".to_owned());
        assert_eq!(mgr.session_count(), 1);

        mgr.approve(&session.session_id, &session.pin, b"signed-cert".to_vec())
            .expect("approve should succeed with the right PIN");

        let outcome = mgr.poll_outcome(&session.session_id).expect("outcome");
        assert_eq!(
            outcome,
            EnrollmentOutcome::Approved {
                signed_certificate: b"signed-cert".to_vec(),
            }
        );
        // Session is consumed after a successful poll — no replay.
        assert_eq!(mgr.session_count(), 0);
        assert!(mgr.poll_outcome(&session.session_id).is_none());
    }

    #[test]
    fn approve_rejects_pin_mismatch() {
        let mgr = EnrollmentManager::new();
        let session = mgr.start_session("nodepubkey".to_owned());
        let err = mgr
            .approve(&session.session_id, "WRONG-PIN", vec![1, 2, 3])
            .expect_err("PIN mismatch should fail");
        assert!(err.contains("PIN mismatch"));
        // The session is still alive after a wrong PIN — operator can retry.
        assert_eq!(mgr.session_count(), 1);
    }

    #[test]
    fn poll_returns_none_while_pending() {
        let mgr = EnrollmentManager::new();
        let session = mgr.start_session("nodepubkey".to_owned());
        assert!(mgr.poll_outcome(&session.session_id).is_none());
    }

    #[test]
    fn rejecting_a_session_surfaces_the_reason_then_consumes() {
        let mgr = EnrollmentManager::new();
        let session = mgr.start_session("nodepubkey".to_owned());
        mgr.reject(&session.session_id, "unrecognized device".to_owned())
            .expect("reject should record outcome");
        let outcome = mgr.poll_outcome(&session.session_id).expect("outcome");
        assert!(matches!(outcome, EnrollmentOutcome::Rejected { .. }));
        assert_eq!(mgr.session_count(), 0);
    }
}
