## 1. Integrity model binding

- [x] 1.1 Preserve dynamic model bindings during integrity normalization
- [x] 1.2 Add normalization and route-authorization regression tests
- [x] 1.3 Resolve advertised `engine/alias` model IDs to runtime aliases

## 2. Local deployment

- [ ] 2.1 Add the uploaded Qwen alias as a dynamic binding on the OpenAI chat route
- [ ] 2.2 Reseal/reload the deployed manifest and verify HTTPS chat behavior
- [x] 2.3 Route `ai.tachyon-mesh.wsl` through HomeLab and remove the old hostname route

## 3. Continue integration

- [x] 3.1 Point Continue at `https://ai.tachyon-mesh.wsl/v1`
- [x] 3.2 Remove the obsolete workstation port-forward and verify model discovery
