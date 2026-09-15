import { beforeEach, describe, expect, it, vi } from "vitest";

import "./TachyonAuthStepCredentials";
import { resilientInvoke } from "../../utils/network";
import type { TachyonAuthStepCredentials } from "./TachyonAuthStepCredentials";

vi.mock("../../utils/network", () => ({
  resilientInvoke: vi.fn(),
}));

// A URL containing a double quote and an event handler, shaped like the
// payload in issue #423: if it ever reaches the `#cred-url` input's HTML
// via string interpolation instead of a property assignment, it breaks out
// of the `value="..."` attribute and adds a live `onfocus` handler.
const MALICIOUS_URL = 'https://x" autofocus onfocus="window.__pwned=1';

function mountCredentials(): TachyonAuthStepCredentials {
  const el = document.createElement("auth-step-credentials") as TachyonAuthStepCredentials;
  document.body.appendChild(el);
  return el;
}

describe("TachyonAuthStepCredentials", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    vi.mocked(resilientInvoke).mockReset();
    vi.mocked(resilientInvoke).mockResolvedValue(null);
  });

  it("setUrl() carries a value with a double quote into the input as data, not markup", async () => {
    const el = mountCredentials();
    // Let the async restoreCustomCa()/restoreCredentials() calls from
    // connectedCallback settle before asserting on final DOM state.
    await Promise.resolve();
    await Promise.resolve();

    el.setUrl(MALICIOUS_URL);

    const input = el.shadowRoot?.getElementById("cred-url") as HTMLInputElement | null;
    expect(input).not.toBeNull();
    expect(input?.value).toBe(MALICIOUS_URL);
    // No attribute-breakout: the quote in the value must not have produced
    // a second attribute on the input (or any other element).
    expect(input?.hasAttribute("autofocus")).toBe(false);
    expect(input?.hasAttribute("onfocus")).toBe(false);
    expect(el.shadowRoot?.querySelector("[onfocus]")).toBeNull();
    expect(el.shadowRoot?.querySelectorAll("#cred-url").length).toBe(1);
  });

  it("survives a re-render (e.g. a language change) without losing or breaking the URL", async () => {
    const el = mountCredentials();
    await Promise.resolve();
    await Promise.resolve();

    el.setUrl(MALICIOUS_URL);
    window.dispatchEvent(new CustomEvent("i18n:language-changed"));

    const input = el.shadowRoot?.getElementById("cred-url") as HTMLInputElement | null;
    expect(input?.value).toBe(MALICIOUS_URL);
    expect(el.shadowRoot?.querySelector("[onfocus]")).toBeNull();
  });

  it("restoreCredentials() from a saved credential also sets the value as data, not markup", async () => {
    vi.mocked(resilientInvoke).mockImplementation(async (command: string) => {
      if (command === "load_credentials") {
        return { url: MALICIOUS_URL, username: "op", password: "" };
      }
      return null;
    });

    const el = mountCredentials();
    // restoreCredentials() runs asynchronously from connectedCallback.
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    const input = el.shadowRoot?.getElementById("cred-url") as HTMLInputElement | null;
    expect(input?.value).toBe(MALICIOUS_URL);
    expect(el.shadowRoot?.querySelector("[onfocus]")).toBeNull();
  });
});
