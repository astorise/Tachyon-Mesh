# Tasks: Change 048 Implementation

**Agent Instruction:** Implement the zero-trust cryptographic identity injection. The host must always overwrite user-provided identity headers.

- [ ] Generate an ephemeral Ed25519 keypair at host startup and inject the public key into system FaaS runtimes.
- [ ] Strip user-provided identity headers from outbound mesh requests and replace them with short-lived host-signed identity tokens.
- [ ] Verify identity tokens in the storage broker and enforce volume-scope ACL checks from the authenticated caller identity.
- [ ] Validate spoofed headers are ignored and out-of-scope writes are rejected with HTTP 403.
