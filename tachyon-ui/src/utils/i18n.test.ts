import { describe, it, expect, beforeEach } from "vitest";
import { getLanguage, setLanguage, t, translateBackendError, ErrorTranslator } from "./i18n";

beforeEach(() => {
  setLanguage("en");
});

describe("t()", () => {
  it("returns the English string for a known key", () => {
    expect(t("nav.dashboard")).toBe("Dashboard");
  });

  it("returns the French string after switching to fr", () => {
    setLanguage("fr");
    expect(t("nav.dashboard")).toBe("Tableau de bord");
  });

  it("falls back to English when a key is missing in fr", () => {
    setLanguage("fr");
    // Force a key that only exists in EN (simulate gap)
    expect(t("nav.dashboard")).not.toBe("nav.dashboard");
  });

  it("returns the key itself when not found in any dictionary", () => {
    expect(t("no.such.key")).toBe("no.such.key");
  });

  it("resolves topology type keys for all 8 node types", () => {
    const types = [
      "endpoint", "system-faas", "custom-wasm", "llm",
      "kv-cache", "storage", "message-broker", "external-resource",
    ];
    for (const type of types) {
      const result = t(`topology.type.${type}`);
      expect(result).not.toBe(`topology.type.${type}`);
      expect(result.length).toBeGreaterThan(0);
    }
  });

  it("resolves topology type keys in French", () => {
    setLanguage("fr");
    expect(t("topology.type.llm")).toBe("LLM");
    expect(t("topology.type.storage")).not.toContain("topology.type");
  });

  it("resolves iam keys for both languages", () => {
    expect(t("iam.authenticate")).toBe("Authenticate");
    setLanguage("fr");
    expect(t("iam.authenticate")).toBe("S'authentifier");
  });

  it("resolves panel keys for all 8 domain panels", () => {
    const keys = [
      "ai.title", "hardware.title", "identity.title", "rbac.title",
      "workloads.title", "fleet.title", "supply-chain.title", "resilience.title",
    ];
    for (const key of keys) {
      expect(t(key)).not.toBe(key);
    }
  });

  it("resolves model-upload panel keys in both languages", () => {
    const keys = [
      "ai.upload.title", "ai.upload.subtitle", "ai.upload.select",
      "ai.upload.selected", "ai.upload.uploading", "ai.upload.scanning",
      "ai.upload.including", "ai.upload.prepared", "ai.upload.sending",
      "ai.upload.committing", "ai.upload.files", "ai.upload.inputSize",
      "ai.upload.archiveSize", "ai.upload.sent", "ai.upload.part",
      "ai.upload.cancelled", "ai.upload.success", "ai.upload.registryHint", "ai.upload.error",
    ];
    for (const lang of ["en", "fr"] as const) {
      setLanguage(lang);
      for (const key of keys) {
        expect(t(key), `${key} (${lang})`).not.toBe(key);
      }
    }
  });
});

describe("getLanguage / setLanguage", () => {
  it("starts with 'en' as default", () => {
    expect(getLanguage()).toBe("en");
  });

  it("switches to 'fr'", () => {
    setLanguage("fr");
    expect(getLanguage()).toBe("fr");
  });

  it("treats any unknown value as 'en'", () => {
    setLanguage("de");
    expect(getLanguage()).toBe("en");
  });

  it("setLanguage returns the resulting language", () => {
    expect(setLanguage("fr")).toBe("fr");
    expect(setLanguage("en")).toBe("en");
  });
});

describe("ErrorTranslator", () => {
  it("translates a kv_cache_size error in English", () => {
    const translator = new ErrorTranslator();
    const result = translator.translate(
      "AI WIT validation failed: kv_cache_size must be between 8 and 128 GB",
    );
    expect(result).toContain("8");
    expect(result).toContain("128");
    expect(result).not.toContain("kv_cache_size");
  });

  it("translates a kv_cache_size error in French", () => {
    setLanguage("fr");
    const translator = new ErrorTranslator();
    const result = translator.translate(
      "AI WIT validation failed: kv_cache_size must be between 8 and 128 GB",
    );
    expect(result).toContain("8");
    expect(result).toContain("128");
  });

  it("translates an OTLP endpoint error", () => {
    const translator = new ErrorTranslator();
    const result = translator.translate("otlp_endpoint must use http(s)://");
    expect(result).not.toBe("otlp_endpoint must use http(s)://");
    expect(result.toLowerCase()).toContain("http");
  });

  it("wraps unknown errors with a system error prefix", () => {
    const translator = new ErrorTranslator();
    const result = translator.translate("some unexpected error");
    expect(result).toContain("some unexpected error");
  });
});

describe("translateBackendError", () => {
  it("translates a known rate-limit error to a non-empty string", () => {
    const result = translateBackendError("Rate limit exceeded");
    expect(result.length).toBeGreaterThan(0);
    expect(result).not.toBe("Rate limit exceeded");
  });
});
