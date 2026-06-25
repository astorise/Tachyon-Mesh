## 1. Integrity model binding

- [x] 1.1 Preserve dynamic model bindings during integrity normalization
- [x] 1.2 Add normalization and route-authorization regression tests
- [x] 1.3 Resolve advertised `engine/alias` model IDs to runtime aliases

## 2. Local deployment

- [x] 2.1 Add the uploaded Qwen alias as a dynamic binding on the OpenAI chat route
- [x] 2.2 Reseal/reload the deployed manifest and verify HTTPS chat authorization behavior
  - The live pod hot-reloaded the signed manifest successfully.
  - `GET /v1/models` returns the uploaded Qwen model.
  - `POST /v1/chat/completions` now passes route authorization and enters model loading; the installed 35B checkpoint did not finish within the 90-second smoke-test window because the deployment is concurrently reporting repeated S3 multipart timeouts. That storage/runtime issue is independent of the sealed-binding defect fixed by this change.
- [x] 2.3 Route `ai.tachyon-mesh.wsl` through HomeLab and remove the old hostname route

## 3. Continue integration

- [x] 3.1 Point Continue at `https://ai.tachyon-mesh.wsl/v1`
- [x] 3.2 Remove the obsolete workstation port-forward and verify model discovery
