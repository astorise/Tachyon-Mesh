#!/usr/bin/env node
// Seal integrity.lock for the zero-touch enrollment E2E: inject the `enrollment`
// policy block (pointing at the test OIDC issuer) and a cluster-CA signer route,
// then re-sign with the same algorithm as scripts/resign-integrity.js
//   signature = Ed25519Sign(SHA256(config_payload_utf8), local_key)
//
// Usage: node scripts/e2e/seal-zero-touch.js <issuer-url> [lock-path]
'use strict';

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const issuer = process.argv[2];
if (!issuer) {
    console.error('usage: seal-zero-touch.js <issuer-url> [lock-path]');
    process.exit(2);
}
const WORKSPACE_ROOT = path.resolve(__dirname, '..', '..');
const LOCK_FILE = process.argv[3] || path.join(WORKSPACE_ROOT, 'integrity.lock');
const KEY_FILE = path.join(WORKSPACE_ROOT, '.tachyon-local-ed25519');
const PKCS8_ED25519_HEADER = Buffer.from('302e020100300506032b657004220420', 'hex');
const SIGNER_ROUTE = '/internal/cert-manager/sign-node';

function loadOrCreateKey() {
    if (fs.existsSync(KEY_FILE)) {
        const raw = fs.readFileSync(KEY_FILE);
        if (raw.length !== 32) throw new Error(`${KEY_FILE} must be 32 bytes`);
        return raw;
    }
    const seed = crypto.randomBytes(32);
    fs.writeFileSync(KEY_FILE, seed, { mode: 0o600 });
    return seed;
}
const importKey = (seed) =>
    crypto.createPrivateKey({ key: Buffer.concat([PKCS8_ED25519_HEADER, seed]), format: 'der', type: 'pkcs8' });
const publicKeyHex = (key) =>
    crypto.createPublicKey(key).export({ type: 'spki', format: 'der' }).slice(12).toString('hex');
const sign = (payload, key) =>
    crypto.sign(null, crypto.createHash('sha256').update(payload, 'utf8').digest(), key).toString('hex');

const lock = JSON.parse(fs.readFileSync(LOCK_FILE, 'utf8'));
const config = JSON.parse(lock.config_payload);

// 1. Enrollment policy: machine-identity auto-approval for the node SA.
config.enrollment = {
    mode: 'both',
    oidc_issuer: issuer,
    oidc_audience: 'tachyon',
    auto_approve_tags: ['namespace=tachyon', 'serviceaccount=tachyon-node'],
};

// 2. Sealed route so node-registry's `http://mesh/internal/cert-manager/sign-node`
//    call resolves to the cert-manager FaaS.
if (!config.routes.some((r) => r.path === SIGNER_ROUTE)) {
    // Cluster-CA seed handed to cert-manager via route env (a WASM guest cannot
    // read an arbitrary host mount; env is passed into the sandbox).
    const caSeed = crypto.randomBytes(32).toString('hex');
    config.routes.push({
        path: SIGNER_ROUTE,
        role: 'system',
        name: 'cert-manager',
        version: '1.0.0',
        min_instances: 0,
        max_concurrency: 16,
        // Explicit target: module resolution uses targets[0].module, NOT the
        // route name — the path's last segment ("sign-node") would mis-resolve.
        targets: [{ module: 'system-faas-cert-manager', weight: 100 }],
        env: { TACHYON_CLUSTER_CA_SEED: caSeed },
        dependencies: {},
    });
}

const payload = JSON.stringify(config);
const key = importKey(loadOrCreateKey());
fs.writeFileSync(
    LOCK_FILE,
    JSON.stringify({ config_payload: payload, public_key: publicKeyHex(key), signature: sign(payload, key) }, null, 2) + '\n',
);
console.log(`Sealed zero-touch integrity.lock (issuer=${issuer}, signer route=${SIGNER_ROUTE})`);
