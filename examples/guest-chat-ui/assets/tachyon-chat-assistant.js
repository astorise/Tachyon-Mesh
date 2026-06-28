const DEFAULT_ENDPOINT = "/ai/v1/chat/completions";
const DEFAULT_MODELS_ENDPOINT = "/ai/v1/models";
const DEFAULT_MODEL = "safetensors/nvidia--Qwen3.6-35B-A3B-NVFP4";

class TachyonChatAssistant extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: "open" });
    this.messages = [
      {
        role: "system",
        content: "You are Tachyon Mesh's local inference assistant. Answer concisely.",
      },
    ];
    this.abortController = null;
  }

  connectedCallback() {
    this.endpoint = this.getAttribute("endpoint") || DEFAULT_ENDPOINT;
    this.modelsEndpoint = this.getAttribute("models-endpoint") || DEFAULT_MODELS_ENDPOINT;
    this.model = this.getAttribute("model") || DEFAULT_MODEL;
    this.render();
    this.loadModels();
  }

  render() {
    this.shadowRoot.innerHTML = `
      <style>
        :host {
          display: block;
          min-height: 100vh;
          background: #f7f7f4;
          color: #181818;
          font: 14px/1.5 Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        }

        .shell {
          display: grid;
          grid-template-rows: auto 1fr auto;
          min-height: 100vh;
        }

        header {
          display: flex;
          align-items: center;
          justify-content: space-between;
          gap: 16px;
          padding: 14px 18px;
          border-bottom: 1px solid #deded8;
          background: #ffffff;
        }

        h1 {
          margin: 0;
          font-size: 16px;
          font-weight: 650;
        }

        select {
          max-width: min(44vw, 420px);
          border: 1px solid #c9c9c2;
          border-radius: 6px;
          padding: 7px 10px;
          background: #ffffff;
          color: #181818;
        }

        .transcript {
          display: flex;
          flex-direction: column;
          gap: 14px;
          overflow: auto;
          padding: 20px max(18px, calc((100vw - 840px) / 2));
        }

        .message {
          display: grid;
          gap: 6px;
          max-width: 760px;
        }

        .message.user {
          align-self: end;
        }

        .role {
          color: #6d6d66;
          font-size: 12px;
          font-weight: 650;
          text-transform: uppercase;
        }

        .bubble {
          white-space: pre-wrap;
          overflow-wrap: anywhere;
          border: 1px solid #deded8;
          border-radius: 8px;
          padding: 11px 13px;
          background: #ffffff;
        }

        .user .bubble {
          background: #202124;
          border-color: #202124;
          color: #ffffff;
        }

        form {
          display: grid;
          grid-template-columns: 1fr auto;
          gap: 10px;
          padding: 14px max(18px, calc((100vw - 840px) / 2));
          border-top: 1px solid #deded8;
          background: #ffffff;
        }

        textarea {
          min-height: 44px;
          max-height: 180px;
          resize: vertical;
          border: 1px solid #c9c9c2;
          border-radius: 8px;
          padding: 11px 12px;
          color: #181818;
          background: #ffffff;
          font: inherit;
        }

        button {
          width: 46px;
          height: 46px;
          border: 0;
          border-radius: 8px;
          background: #0f766e;
          color: white;
          cursor: pointer;
          font-size: 20px;
          line-height: 1;
        }

        button:disabled {
          background: #9ca3af;
          cursor: progress;
        }

        .status {
          color: #6d6d66;
          font-size: 12px;
          min-height: 18px;
          padding: 0 max(18px, calc((100vw - 840px) / 2)) 12px;
          background: #ffffff;
        }
      </style>
      <section class="shell">
        <header>
          <h1>Tachyon Chat</h1>
          <select aria-label="Model" part="model-select">
            <option value="${escapeHtml(this.model)}">${escapeHtml(this.model)}</option>
          </select>
        </header>
        <div class="transcript" part="transcript"></div>
        <div>
          <form part="composer">
            <textarea aria-label="Message" placeholder="Message Tachyon" rows="1"></textarea>
            <button aria-label="Send" title="Send">↑</button>
          </form>
          <div class="status" part="status"></div>
        </div>
      </section>
    `;

    this.transcript = this.shadowRoot.querySelector(".transcript");
    this.status = this.shadowRoot.querySelector(".status");
    this.textarea = this.shadowRoot.querySelector("textarea");
    this.button = this.shadowRoot.querySelector("button");
    this.select = this.shadowRoot.querySelector("select");
    this.shadowRoot.querySelector("form").addEventListener("submit", (event) => {
      event.preventDefault();
      this.send();
    });
    this.textarea.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        this.send();
      }
    });
    this.renderMessages();
  }

  async loadModels() {
    try {
      const response = await fetch(this.modelsEndpoint, { headers: { accept: "application/json" } });
      if (!response.ok) {
        return;
      }
      const payload = await response.json();
      const models = Array.isArray(payload.data) ? payload.data.map((item) => item.id).filter(Boolean) : [];
      if (models.length === 0) {
        return;
      }
      this.select.innerHTML = models
        .map((id) => `<option value="${escapeHtml(id)}">${escapeHtml(id)}</option>`)
        .join("");
      this.select.value = models.includes(this.model) ? this.model : models[0];
      this.model = this.select.value;
      this.select.addEventListener("change", () => {
        this.model = this.select.value;
      });
    } catch {
      this.setStatus("Model list unavailable; using the configured model.");
    }
  }

  async send() {
    const content = this.textarea.value.trim();
    if (!content || this.abortController) {
      return;
    }

    this.textarea.value = "";
    this.messages.push({ role: "user", content });
    const assistant = { role: "assistant", content: "" };
    this.messages.push(assistant);
    this.renderMessages();
    this.setBusy(true);
    this.setStatus("Streaming response...");
    this.abortController = new AbortController();

    try {
      const response = await fetch(this.endpoint, {
        method: "POST",
        headers: {
          accept: "text/event-stream",
          "content-type": "application/json",
        },
        body: JSON.stringify({
          model: this.model,
          messages: this.messages.filter((message) => message.role !== "assistant" || message.content),
          stream: true,
        }),
        signal: this.abortController.signal,
      });

      if (!response.ok) {
        const text = await response.text();
        throw new Error(text || `HTTP ${response.status}`);
      }

      await readOpenAiStream(response, (delta) => {
        assistant.content += delta;
        this.renderMessages();
      });
      this.setStatus("");
    } catch (error) {
      assistant.content = `Request failed: ${error.message}`;
      this.renderMessages();
      this.setStatus("Gateway request failed.");
    } finally {
      this.abortController = null;
      this.setBusy(false);
    }
  }

  renderMessages() {
    if (!this.transcript) {
      return;
    }
    const visible = this.messages.filter((message) => message.role !== "system");
    this.transcript.innerHTML = visible
      .map((message) => `
        <article class="message ${message.role}">
          <div class="role">${escapeHtml(message.role)}</div>
          <div class="bubble">${escapeHtml(message.content || " ")}</div>
        </article>
      `)
      .join("");
    this.transcript.scrollTop = this.transcript.scrollHeight;
  }

  setBusy(isBusy) {
    this.button.disabled = isBusy;
    this.textarea.disabled = isBusy;
  }

  setStatus(message) {
    if (this.status) {
      this.status.textContent = message;
    }
  }
}

async function readOpenAiStream(response, onDelta) {
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    buffer += decoder.decode(value, { stream: true });
    const frames = buffer.split("\n\n");
    buffer = frames.pop() || "";
    for (const frame of frames) {
      for (const line of frame.split("\n")) {
        if (!line.startsWith("data:")) {
          continue;
        }
        const data = line.slice(5).trim();
        if (!data || data === "[DONE]") {
          continue;
        }
        const parsed = JSON.parse(data);
        const delta = parsed.choices?.[0]?.delta?.content;
        if (delta) {
          onDelta(delta);
        }
      }
    }
  }
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

customElements.define("tachyon-chat-assistant", TachyonChatAssistant);
