## 1. Legacy Mock Service

- [x] 1.1 Add a `legacy-mock` workspace member with an Axum server listening on port `8081`.
- [x] 1.2 Implement `GET /ping` returning `legacy_ok` and `POST /call-faas` using `FAAS_URL` to call the FaaS service.

## 2. Host Mesh Bridge

- [x] 2.1 Add a `guest-call-legacy` WASI guest that emits `MESH_FETCH:http://legacy-service:8081/ping`.
- [x] 2.2 Update `core-host` to accept both `GET` and `POST` requests and resolve `MESH_FETCH:` guest output through outbound HTTP fetches.

## 3. Packaging and Deployment

- [x] 3.1 Update the workspace container build to compile both guest modules, build `legacy-mock`, and generate `integrity.lock` with both sealed routes.
- [x] 3.2 Extend `manifests/deploy.yaml` to deploy `tachyon-host` and `legacy-deployment` with services on ports `8080` and `8081`.

## 4. Hybrid Mesh Verification

- [x] 4.1 Expand `.github/workflows/integration.yml` to build and import both images and validate the four direct and bridged traffic paths.
- [x] 4.2 Verify the change with `cargo test --workspace` and `openspec validate --all`.
