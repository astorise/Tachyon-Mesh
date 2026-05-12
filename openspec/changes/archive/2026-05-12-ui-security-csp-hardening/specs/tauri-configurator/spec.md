# Technical Specification: CSP and DOM Sanitization

## 1. Content Security Policy (Tauri Configuration)
Update `tachyon-ui/tauri.conf.json` to enforce the following policy. This allows standard Tauri IPC, WebSockets for telemetry, and HTTP requests to the core-host, while blocking inline scripts and evals.

```json
{
  "tauri": {
    "security": {
      "csp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src ipc: [http://ipc.localhost](http://ipc.localhost) tauri: 'self' http://localhost:* ws://localhost:* wss://localhost:*; img-src 'self' asset: data:;"
    }
  }
}
```
*Note: `style-src 'unsafe-inline'` is temporarily maintained to accommodate Tailwind v4 dynamic classes and GSAP, but `script-src` is strictly locked to `'self'`.*

## 2. DOM Manipulation Refactoring
The primary target is `tachyon-ui/src/components/iam/TachyonIAM.ts`, but this applies globally.

**Anti-Pattern (To be removed):**
```typescript
// DANGEROUS
this.container.innerHTML = `
  <div class="cluster-info">
    Connected to: ${clusterUrl}
  </div>
`;
```

**Safe Pattern (To be implemented):**
```typescript
// SAFE - Option A: textContent
const clusterInfo = document.createElement('div');
clusterInfo.className = 'cluster-info';
clusterInfo.textContent = `Connected to: ${clusterUrl}`;
this.container.appendChild(clusterInfo);

// SAFE - Option B: Template string without user data + text node injection
this.container.innerHTML = `<div class="cluster-info">Connected to: </div>`;
this.container.querySelector('.cluster-info').appendChild(document.createTextNode(clusterUrl));
```

## 3. Tooling & Enforcement
Update `tachyon-ui/.eslintrc.json` or equivalent linter (if applicable) to throw an error on `.innerHTML` usage:
```json
"rules": {
  "no-restricted-properties": [
    2,
    {
      "property": "innerHTML",
      "message": "Use textContent or safe DOM manipulation instead of innerHTML to prevent XSS."
    }
  ]
}
```