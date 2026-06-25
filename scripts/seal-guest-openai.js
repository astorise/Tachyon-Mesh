#!/usr/bin/env node
// Adds the guest-openai user routes (the OpenAI-compatible API surface:
// /ai/v1/models, /ai/v1/chat/completions, /internal/guest-openai/register) into
// integrity.lock and re-signs it with the local Ed25519 key, so a fresh
// deployment seeds these routes and `GET /ai/v1/models` is served. Idempotent.
'use strict';
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const ROOT = path.resolve(__dirname, '..');
const LOCK = path.join(ROOT, 'integrity.lock');
const KEY = path.join(ROOT, '.tachyon-local-ed25519');
const SRC = path.join(ROOT, 'examples', 'guest-examples', 'manifest.json');
const PKCS8 = Buffer.from('302e020100300506032b657004220420', 'hex');

const seed = fs.readFileSync(KEY);
if (seed.length !== 32) throw new Error(`key must be 32 bytes, got ${seed.length}`);
const priv = crypto.createPrivateKey({ key: Buffer.concat([PKCS8, seed]), format: 'der', type: 'pkcs8' });
const pubHex = crypto.createPublicKey(priv).export({ type: 'spki', format: 'der' }).slice(12).toString('hex');

const lock = JSON.parse(fs.readFileSync(LOCK, 'utf8'));
console.log('current pubkey   :', lock.public_key.slice(0, 16) + '…');
console.log('local key pubkey :', pubHex.slice(0, 16) + '…', lock.public_key.toLowerCase() === pubHex.toLowerCase() ? '(MATCH)' : '(DIFFERENT)');

const config = JSON.parse(lock.config_payload);
const src = JSON.parse(fs.readFileSync(SRC, 'utf8'));
const legacyV1 = process.env.TACHYON_OPENAI_LEGACY_V1 === '1';
const openaiRoutes = src.routes
  .filter(r => (r.targets || []).some(t => t.module === 'guest-openai'))
  .map(route => legacyV1
    ? { ...route, path: route.path.replace(/^\/ai\/v1\//, '/v1/') }
    : route);
const isOpenAiRoute = route =>
  (route.targets || []).some(target => target.module === 'guest-openai');
const updated = config.routes.filter(isOpenAiRoute).map(route => route.path);
config.routes = config.routes.filter(route => !isOpenAiRoute(route));
config.routes.push(...openaiRoutes);
const added = openaiRoutes.map(route => route.path);
console.log('guest-openai routes in source:', openaiRoutes.map(r => r.path).join(', '));
console.log('updated in integrity.lock    :', updated.length ? updated.join(', ') : '(none)');
console.log('added to integrity.lock      :', added.length ? added.join(', ') : '(none — already present)');

const payload = JSON.stringify(config);
const sig = crypto.sign(null, crypto.createHash('sha256').update(payload, 'utf8').digest(), priv).toString('hex');
fs.writeFileSync(LOCK, JSON.stringify({ config_payload: payload, public_key: pubHex, signature: sig }, null, 2) + '\n');
console.log('routes now sealed:', config.routes.map(r => r.path).join(', '));
console.log('signed integrity.lock written.');
