# Implementation Tasks

## Phase 1: Core Host Boot Branching
- [ ] Branching the boot path so an unenrolled `core-host` opens an
      outbound long-poll instead of running the regular request server is
      a Session C item — it requires the secure overlay (Noise / mTLS) for
      the cert-delivery channel. The host-side primitives needed to drive
      that flow now exist (see Phase 2/3 below).

## Phase 2: Enrollment FaaS / Manager
- [x] New `core-host/src/node_enrollment.rs` module owns the
      operator-side state machine with `EnrollmentManager` (pending
      sessions, PIN generation from a read-friendly alphabet without
      `O/0/I/1`, 15-minute TTL).
- [x] PINs are minted via `rand` from a 32-char alphabet over 6 chars,
      formatted as `XXX-XXX` for clarity when read aloud.
- [x] `EnrollmentManager::approve` requires PIN match; mismatched PINs
      keep the session alive so the operator can retry without
      restarting the device.

## Phase 3: Cluster-side Approval Endpoints
- [x] `POST /admin/enrollment/start { nodePublicKey }` →
      `{ sessionId, pin }` — invoked by the unenrolled node's outbound
      channel against any active mesh node (the active node's
      `admin_auth_middleware` gates access on the operator side).
- [x] `POST /admin/enrollment/approve { sessionId, pin, signedCertificateHex }`
      — operator-driven approval entered via Tachyon Studio.
- [x] `GET /admin/enrollment/poll/{session_id}` — invoked by the
      unenrolled node; returns 204 while pending, 200 with the cert
      bytes once approved, 410 Gone after rejection. The session is
      consumed by a successful poll so it cannot be replayed.

## Phase 4: Validation
- [x] 5 module unit tests cover PIN format, approve+poll round trip,
      PIN mismatch, pending-poll, and reject-then-poll semantics.
- [x] 1 handler integration test covers the full
      start → wrong-PIN-rejected → approve → poll → consumed flow.
- [ ] End-to-end "spin up Node B clean, pair from Node A, watch Node B
      join the mesh" is left for Session C, which adds the secure
      delivery side of the flow.
