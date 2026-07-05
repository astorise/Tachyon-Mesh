#!/usr/bin/env node
// Adds the bench regression routes (the guest-chain-a/b/c 3-hop chain used by
// bench/workloads/faas-chain.yaml, plus guest-example used by
// bench/workloads/cold-start.yaml) into integrity.lock and re-signs it with
// the local Ed25519 key, so a bench-only core-host image serves them. Only
// meant to be run against an ephemeral CI checkout right before building the
// bench Docker image (see .github/workflows/bench-regression.yml) — never
// commit the resulting integrity.lock. Idempotent.
'use strict';
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const ROOT = path.resolve(__dirname, '..');
const LOCK = path.join(ROOT, 'integrity.lock');
const KEY = path.join(ROOT, '.tachyon-local-ed25519');
const SRC = path.join(ROOT, 'examples', 'guest-examples', 'manifest.json');
const PKCS8 = Buffer.from('302e020100300506032b657004220420', 'hex');

function loadOrCreateKey() {
  if (fs.existsSync(KEY)) {
    const raw = fs.readFileSync(KEY);
    if (raw.length !== 32) throw new Error(`${KEY} must be 32 bytes, got ${raw.length}`);
    return raw;
  }
  const seed = crypto.randomBytes(32);
  fs.writeFileSync(KEY, seed, { mode: 0o600 });
  console.log('Generated new signing key -> .tachyon-local-ed25519');
  return seed;
}

const seed = loadOrCreateKey();
const priv = crypto.createPrivateKey({ key: Buffer.concat([PKCS8, seed]), format: 'der', type: 'pkcs8' });
const pubHex = crypto.createPublicKey(priv).export({ type: 'spki', format: 'der' }).slice(12).toString('hex');

const lock = JSON.parse(fs.readFileSync(LOCK, 'utf8'));
console.log('current pubkey   :', lock.public_key.slice(0, 16) + '…');
console.log('local key pubkey :', pubHex.slice(0, 16) + '…', lock.public_key.toLowerCase() === pubHex.toLowerCase() ? '(MATCH)' : '(DIFFERENT)');

const config = JSON.parse(lock.config_payload);
const src = JSON.parse(fs.readFileSync(SRC, 'utf8'));

const isBenchRoute = route => {
  const name = route.name || '';
  const targets = (route.targets || []).map(t => t.module || '');
  return name.startsWith('guest-chain-')
    || name === 'guest-example'
    || targets.some(m => m.startsWith('guest-chain-') || m === 'guest-example');
};

const benchRoutes = src.routes.filter(isBenchRoute);
const updated = config.routes.filter(isBenchRoute).map(r => r.path);
config.routes = config.routes.filter(route => !isBenchRoute(route));
config.routes.push(...benchRoutes);
const added = benchRoutes.map(r => r.path);
console.log('bench routes in source    :', benchRoutes.map(r => r.path).join(', '));
console.log('updated in integrity.lock :', updated.length ? updated.join(', ') : '(none)');
console.log('added to integrity.lock   :', added.length ? added.join(', ') : '(none — already present)');

const payload = JSON.stringify(config);
const sig = crypto.sign(null, crypto.createHash('sha256').update(payload, 'utf8').digest(), priv).toString('hex');
fs.writeFileSync(LOCK, JSON.stringify({ config_payload: payload, public_key: pubHex, signature: sig }, null, 2) + '\n');
console.log('routes now sealed:', config.routes.map(r => r.path).join(', '));
console.log('signed integrity.lock written.');
