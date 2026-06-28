import { tachyonSharedStylesheet } from "../../styles/shared-sheets";
import { t } from "../../utils/i18n";

const CHAT_SCRIPT_URL = "/chat/tachyon-chat-assistant.js";
const CHAT_ELEMENT = "tachyon-chat-assistant";

export class TachyonChatPanel extends HTMLElement {
  private readonly root: ShadowRoot;
  private readonly onLanguageChanged = () => {
    this.render();
    void this.loadWidget();
  };

  constructor() {
    super();
    this.root = this.attachShadow({ mode: "open" });
    this.root.adoptedStyleSheets = [tachyonSharedStylesheet];
  }

  connectedCallback(): void {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    void this.loadWidget();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private render(): void {
    this.root.innerHTML = `
      <section class="flex min-h-[calc(100vh-8rem)] flex-col gap-4 text-slate-300">
        <header class="flex flex-col gap-1">
          <h2 class="text-3xl font-light text-cyan-400">${t("chat.title")}</h2>
          <p class="text-xs font-mono text-slate-500">${t("chat.subtitle")}</p>
        </header>
        <div id="chat-host" class="min-h-0 flex-1 overflow-hidden rounded border border-slate-800 bg-slate-950">
          <div id="chat-status" class="flex h-full min-h-96 items-center justify-center px-4 text-center text-sm text-slate-500">
            ${t("chat.loading")}
          </div>
        </div>
      </section>
    `;
  }

  private async loadWidget(): Promise<void> {
    const host = this.root.getElementById("chat-host");
    if (!host) {
      return;
    }

    try {
      if (!customElements.get(CHAT_ELEMENT)) {
        await import(/* @vite-ignore */ CHAT_SCRIPT_URL);
      }
      host.replaceChildren(document.createElement(CHAT_ELEMENT));
    } catch {
      const fallback = document.createElement("div");
      fallback.className =
        "flex h-full min-h-96 flex-col items-center justify-center gap-3 px-4 text-center text-sm text-slate-500";
      const message = document.createElement("p");
      message.textContent = t("chat.unavailable");
      const retry = document.createElement("button");
      retry.type = "button";
      retry.className =
        "rounded border border-slate-700 bg-slate-900 px-3 py-2 text-xs font-semibold text-slate-300 hover:bg-slate-800";
      retry.textContent = t("chat.retry");
      retry.addEventListener("click", () => {
        fallback.replaceChildren();
        void this.loadWidget();
      });
      fallback.append(message, retry);
      host.replaceChildren(fallback);
    }
  }
}

customElements.define("tachyon-chat-panel", TachyonChatPanel);
