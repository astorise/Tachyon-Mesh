# Design: UI Security — CSP Hardening and DOM Sanitization

## Approach

Four independent changes targeting the Tauri WebView security surface.

### 1. Content Security Policy (`tauri.conf.json`)

Replaces `"csp": null` with a strict policy:

```
default-src 'self';
script-src 'self';
style-src 'self' 'unsafe-inline';
connect-src ipc: http://ipc.localhost tauri: 'self' http://localhost:* ws://localhost:* wss://localhost:*;
img-src 'self' asset: data:;
```

Key decisions:
- **`script-src 'self'`** — blocks all inline scripts and evals; Vite bundles everything into self-hosted files so no inline scripts are needed.
- **`style-src 'unsafe-inline'`** — kept because Tailwind v4 emits inline `style` blocks for dynamic utility classes, and GSAP applies inline transforms. Removing it would require a Tailwind migration to static extraction.
- **`connect-src data:`** not needed — images from QR codes are now data URIs served through `img-src data:`.
- **`img-src data:`** — required for the `QRCode.toDataURL()` PNG output.

### 2. QR Code Rendering (`TachyonIAM.ts`)

Both `qr.innerHTML = await QRCode.toString(..., { type: "svg" })` calls are replaced with:
```typescript
const dataUrl = await QRCode.toDataURL(payload, options);
const img = document.createElement("img");
img.src = dataUrl;
qr.replaceChildren(img);
```

**Why**: An SVG string set via `innerHTML` can carry inline event handlers (e.g., `<svg onload="...">`) if the QR payload is crafted to escape the QRCode library's encoding. The library's SVG serialiser does not sanitize the input for HTML injection. A PNG data URL set as `img.src` cannot carry executable content — the browser decodes it as pixels only. `replaceChildren()` is used instead of `innerHTML` to avoid any parser ambiguity.

### 3. AppShell nav links (`TachyonAppShell.ts`)

The `configLinks` template interpolates `entry.route` as an attribute value and `entry.label` (or its i18n override) as text content. Both are escaped through a local `escapeHtml()` helper before being inserted into the `innerHTML` template. The values are currently hardcoded internal data, but defensive escaping prevents regressions if the `ComponentRegistry` ever becomes dynamic.

### 4. ESLint Enforcement (`.eslintrc.json`)

A new `no-restricted-properties` rule flags any future `innerHTML` or `outerHTML` assignment at the linter level, with a message pointing to the approved alternatives (`textContent`, `replaceChildren`, or explicit `escapeHtml` wrapping). The rule is set at severity `2` (error), blocking CI when ESLint is added as a build gate.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| QR format | `toDataURL()` PNG | SVG via `innerHTML` | PNG data URI cannot carry script payloads |
| `style-src` | `'unsafe-inline'` kept | `'nonce-…'` | Tailwind v4 dynamic utilities require inline styles; nonce-based approach requires Vite plugin changes |
| AppShell escaping | `escapeHtml()` inline | DOM node construction | Template is 200+ lines; full rewrite would diverge from the established render pattern |
| ESLint format | `.eslintrc.json` legacy | `eslint.config.js` flat | No existing ESLint setup; legacy format is simpler to add without dependency changes |
