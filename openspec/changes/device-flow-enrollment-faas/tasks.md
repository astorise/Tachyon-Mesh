# Implementation Tasks

## Phase 1: Core Host Boot Branching
- [ ] Implement the check for existing credentials on host startup.
- [ ] Create the "Bootstrap Mode" execution path that strictly isolates and runs the enrollment FaaS.

## Phase 2: Enrollment FaaS
- [ ] Create `systems/system-faas-enrollment`.
- [ ] Implement the cryptographic key generation (via WASI or host calls).
- [ ] Implement the Outbound HTTP client logic to hold a connection open with the cluster.
- [ ] Output the temporary PIN clearly to stdout.

## Phase 3: Cluster-side Approval Endpoint
- [x] Add the `/api/enroll` endpoint on standard nodes to accept inbound connections from pending nodes.
- [x] Add the `/api/admin/approve-node` endpoint to accept the PIN from the UI, sign the certificate, and return it.

## Phase 4: Validation
- [ ] **End-to-End Pair:** Start Node A (Cluster). Start Node B (Clean state). 
- [ ] Verify Node B logs a PIN and pauses. 
- [ ] Send the approval API call to Node A with Node B's PIN. 
- [ ] Verify Node B receives the certificate, exits bootstrap mode, and successfully joins the mesh.
