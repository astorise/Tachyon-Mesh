#!/usr/bin/env node
// Re-signs integrity.lock after removing example guest-* routes.
//
// Usage: node scripts/resign-integrity.js
//
// Signing algorithm matches tachyon-client's sign_manifest_payload:
//   signature = Ed25519Sign(SHA256(config_payload_utf8), local_key)
//
// The local key is loaded from .tachyon-local-ed25519 (32-byte raw seed) or
// generated fresh and saved there for future use.
'use strict';

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const WORKSPACE_ROOT = path.resolve(__dirname, '..');
const LOCK_FILE = path.join(WORKSPACE_ROOT, 'integrity.lock');
const KEY_FILE = path.join(WORKSPACE_ROOT, '.tachyon-local-ed25519');

// PKCS#8 DER header for Ed25519 private key (seed).
// Wraps a raw 32-byte seed into the format Node.js crypto expects.
const PKCS8_ED25519_HEADER = Buffer.from('302e020100300506032b657004220420', 'hex');

function loadOrCreateKey() {
    if (fs.existsSync(KEY_FILE)) {
        const raw = fs.readFileSync(KEY_FILE);
        if (raw.length !== 32) throw new Error(`${KEY_FILE} must be 32 bytes, got ${raw.length}`);
        console.log('Loaded existing signing key from .tachyon-local-ed25519');
        return raw;
    }
    const seed = crypto.randomBytes(32);
    fs.writeFileSync(KEY_FILE, seed, { mode: 0o600 });
    console.log('Generated new signing key → .tachyon-local-ed25519');
    return seed;
}

function importPrivateKey(seed32) {
    const der = Buffer.concat([PKCS8_ED25519_HEADER, seed32]);
    return crypto.createPrivateKey({ key: der, format: 'der', type: 'pkcs8' });
}

function publicKeyHex(privateKey) {
    // SPKI DER for Ed25519: 12-byte header + 32-byte key
    const spki = crypto.createPublicKey(privateKey).export({ type: 'spki', format: 'der' });
    return spki.slice(12).toString('hex');
}

function sign(payload, privateKey) {
    const hash = crypto.createHash('sha256').update(payload, 'utf8').digest();
    return crypto.sign(null, hash, privateKey).toString('hex');
}

function isExampleRoute(route) {
    const p = route.path || '';
    const name = route.name || '';
    const targets = (route.targets || []).map(t => t.module || '');
    return p.startsWith('/api/guest-')
        || name.startsWith('guest-')
        || targets.some(m => m.startsWith('guest-'));
}

// ── Main ──────────────────────────────────────────────────────────────────────

const lock = JSON.parse(fs.readFileSync(LOCK_FILE, 'utf8'));
const config = JSON.parse(lock.config_payload);

const before = config.routes.length;
const removed = config.routes.filter(isExampleRoute).map(r => r.path);
config.routes = config.routes.filter(r => !isExampleRoute(r));
console.log(`Removed ${removed.length} example routes: ${removed.join(', ')}`);
console.log(`Remaining routes: ${config.routes.map(r => r.path).join(', ')}`);

const newPayload = JSON.stringify(config);
const seed = loadOrCreateKey();
const privKey = importPrivateKey(seed);

const newLock = {
    config_payload: newPayload,
    public_key: publicKeyHex(privKey),
    signature: sign(newPayload, privKey),
};

fs.writeFileSync(LOCK_FILE, JSON.stringify(newLock, null, 2) + '\n');
console.log(`\nSealed integrity.lock  public_key=${newLock.public_key.slice(0, 16)}…`);
console.log('Run `cargo build -p core-host` to embed the updated manifest.');
