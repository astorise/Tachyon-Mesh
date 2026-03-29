# Proposal: Change 019 - Just-In-Time Secrets Vault

## Why
Injecting runtime secrets through environment variables makes every dependency in the guest process a potential exfiltration path. Tachyon Mesh already seals route metadata in `integrity.lock`, so the next step is to keep secrets in host memory and only disclose them through an explicit, capability-checked interface.

## What Changes
- Add a `secrets-vault` feature to `core-host`.
- Extend the shared WIT contract with a `secrets-vault` import that component guests can call to fetch named secrets.
- Keep secrets out of the WASI environment block and return them only through the typed host binding.
- Extend the sealed manifest format so user routes can declare optional `allowed_secrets`.
- Extend `tachyon-cli generate` with `--secret-route /path=NAME[,NAME]` so signed manifests can grant secrets to specific routes.
- Update `guest-example` to prove both sides of the contract: the secret is absent from `std::env`, but available through the WIT vault when granted.

## Impact
- Authorized routes can resolve secrets such as `DB_PASS` from host memory when `core-host` is compiled with `--features secrets-vault`.
- Unauthorized routes receive `permission-denied`.
- Builds without the feature keep the import wired but return `vault-disabled`, preserving guest compatibility while keeping the runtime vault inert.
