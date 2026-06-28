# guest-chat-ui

Static FaaS chat UI example for the OpenAI-compatible Tachyon gateway.

The guest serves a framework-free Web Component from `/chat`:

- `GET /chat` returns a small HTML shell.
- `GET /chat/tachyon-chat-assistant.js` returns `<tachyon-chat-assistant>`.
- `GET /chat/styles.css` returns optional page-level styling for the shell.

The component is self-contained in Shadow DOM and calls the browser-visible
OpenAI-compatible gateway directly. By default it posts streamed requests to
`/ai/v1/chat/completions`; override this with attributes:

```html
<tachyon-chat-assistant
  endpoint="/ai/v1/chat/completions"
  models-endpoint="/ai/v1/models"
  model="safetensors/nvidia--Qwen3.6-35B-A3B-NVFP4">
</tachyon-chat-assistant>
```

Deploy `manifest.json` together with `guest-openai` to run the complete demo
stack: OpenAI-compatible gateway, shared model registry, and static chat UI.
