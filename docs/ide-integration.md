# IDE Integration & Schema Validation

Tachyon Mesh dynamically serves JSON Schema documents for its configuration files while `core-host` is running. Binding these to your IDE gives real-time validation and autocompletion — no copying `.schema.json` files needed.

## Prerequisites

`core-host` must be running on `http://127.0.0.1:8080` (default). The schema endpoints are protected by the admin bearer token middleware, but most IDEs fetch schemas once at startup with no authentication required for `GET` schema routes.

Available schema endpoints:

| URL | Describes |
|---|---|
| `http://127.0.0.1:8080/admin/schema/manifest` | `IntegrityConfig` — the manifest format for `POST /admin/manifest` |
| `http://127.0.0.1:8080/admin/schema/integrity-lock` | `integrity.lock` — the sealed lock file format |
| `http://127.0.0.1:8080/admin/schema/openapi.json` | Full OpenAPI 3.1 spec for all admin endpoints |

---

## VS Code

### JSON files (`integrity.lock`)

Add the following to your workspace `.vscode/settings.json`:

```json
{
  "json.schemas": [
    {
      "fileMatch": ["**/integrity.lock"],
      "url": "http://127.0.0.1:8080/admin/schema/integrity-lock"
    },
    {
      "fileMatch": ["**/tachyon-manifest.json"],
      "url": "http://127.0.0.1:8080/admin/schema/manifest"
    }
  ]
}
```

### YAML manifests

If you have the [Red Hat YAML extension](https://marketplace.visualstudio.com/items?itemName=redhat.vscode-yaml) installed, add a modeline comment at the top of any manifest YAML file:

```yaml
# yaml-language-server: $schema=http://127.0.0.1:8080/admin/schema/manifest
```

For Kubernetes deploy manifests (`manifests/deploy.yaml`, `manifests/deploy-gpu-homelab.yaml`) no schema binding is needed — VS Code's built-in Kubernetes schema handles those.

### REST Client / Thunder Client

You can explore the full API with the generated OpenAPI spec:

1. Install the [OpenAPI (Swagger) Viewer](https://marketplace.visualstudio.com/items?itemName=Arjun.swagger-viewer) extension.
2. Open Command Palette → **Preview Swagger** → enter `http://127.0.0.1:8080/admin/schema/openapi.json`.

Or simply open `http://127.0.0.1:8080/admin/docs` in a browser for the embedded Swagger UI.

---

## JetBrains (IntelliJ IDEA, RustRover, CLion)

### JSON Schema Mappings

1. Open **Settings** → **Languages & Frameworks** → **Schemas and DTDs** → **JSON Schema Mappings**.
2. Click **+** and fill in:
   - **Name**: `Tachyon integrity.lock`
   - **Schema URL**: `http://127.0.0.1:8080/admin/schema/integrity-lock`
   - **Schema version**: JSON Schema version 7
   - **File path pattern**: `integrity.lock`
3. Click **+** again for the manifest schema:
   - **Schema URL**: `http://127.0.0.1:8080/admin/schema/manifest`
   - **File path pattern**: `*.manifest.json`

### HTTP Client

JetBrains IDEs have a built-in HTTP Client. Create an `api.http` file to explore endpoints:

```http
### List current manifest
GET http://127.0.0.1:8080/admin/manifest
Authorization: Bearer {{admin_token}}
Accept: application/json

### Fetch OpenAPI schema
GET http://127.0.0.1:8080/admin/schema/openapi.json
Accept: application/json
```

---

## Neovim / LSP

If you use [SchemaStore.nvim](https://github.com/b0o/SchemaStore.nvim) or configure `jsonls` directly:

```lua
-- In your LSP config (e.g. nvim-lspconfig)
require("lspconfig").jsonls.setup({
  settings = {
    json = {
      schemas = {
        {
          fileMatch = { "integrity.lock" },
          url = "http://127.0.0.1:8080/admin/schema/integrity-lock",
        },
      },
    },
  },
})
```

---

## Offline / Air-Gapped Environments

If you cannot reach a running `core-host`, fetch the schemas once and commit them:

```bash
curl -s http://127.0.0.1:8080/admin/schema/integrity-lock > .schemas/integrity-lock.json
curl -s http://127.0.0.1:8080/admin/schema/manifest       > .schemas/manifest.json
```

Then point your IDE configuration to the local paths instead of the HTTP URLs.
