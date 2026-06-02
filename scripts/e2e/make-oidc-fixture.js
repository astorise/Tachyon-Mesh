#!/usr/bin/env node
// Generate a self-contained OIDC fixture for the zero-touch enrollment E2E:
//   - an RSA keypair,
//   - a JWKS (RS256, kid "e2e-key"),
//   - an OIDC discovery document (issuer + jwks_uri),
//   - a signed RS256 JWT carrying k8s-style ServiceAccount claims.
//
// Usage: node scripts/e2e/make-oidc-fixture.js <issuer-url> <out-dir>
// Writes: <out-dir>/{jwks.json, openid-configuration.json, token.jwt}
'use strict';

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const issuer = process.argv[2];
const outDir = process.argv[3] || '.';
if (!issuer) {
    console.error('usage: make-oidc-fixture.js <issuer-url> <out-dir>');
    process.exit(2);
}
fs.mkdirSync(outDir, { recursive: true });

const KID = 'e2e-key';
const b64url = (input) => Buffer.from(input).toString('base64url');

const { privateKey, publicKey } = crypto.generateKeyPairSync('rsa', { modulusLength: 2048 });

// JWKS — node exports n/e as base64url, matching the Rust verifier.
const jwk = publicKey.export({ format: 'jwk' });
jwk.kid = KID;
jwk.alg = 'RS256';
jwk.use = 'sig';
const jwks = { keys: [jwk] };

const openid = {
    issuer,
    jwks_uri: `${issuer}/jwks`,
    response_types_supported: ['id_token'],
    subject_types_supported: ['public'],
    id_token_signing_alg_values_supported: ['RS256'],
};

const now = Math.floor(Date.now() / 1000);
const header = { alg: 'RS256', typ: 'JWT', kid: KID };
const claims = {
    iss: issuer,
    sub: 'system:serviceaccount:tachyon:tachyon-node',
    aud: ['tachyon'],
    iat: now,
    exp: now + 3600,
    'kubernetes.io': {
        namespace: 'tachyon',
        serviceaccount: { name: 'tachyon-node' },
    },
};
const signingInput = `${b64url(JSON.stringify(header))}.${b64url(JSON.stringify(claims))}`;
// crypto.sign('sha256', …) on an RSA key produces RSASSA-PKCS1-v1_5 (RS256).
const signature = crypto.sign('sha256', Buffer.from(signingInput), privateKey).toString('base64url');
const jwt = `${signingInput}.${signature}`;

fs.writeFileSync(path.join(outDir, 'jwks.json'), JSON.stringify(jwks));
fs.writeFileSync(path.join(outDir, 'openid-configuration.json'), JSON.stringify(openid));
fs.writeFileSync(path.join(outDir, 'token.jwt'), jwt);

console.log(`OIDC fixture written to ${outDir} (issuer=${issuer}, kid=${KID})`);
