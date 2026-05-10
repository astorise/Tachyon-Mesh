import { vi } from "vitest";

// happy-dom provides localStorage but it may not be fully wired on some vitest
// versions; ensure a consistent implementation across environments.
const store: Record<string, string> = {};
const localStorageMock = {
  getItem: (key: string) => store[key] ?? null,
  setItem: (key: string, value: string) => {
    store[key] = value;
  },
  removeItem: (key: string) => {
    delete store[key];
  },
  clear: () => {
    for (const key of Object.keys(store)) delete store[key];
  },
  get length() {
    return Object.keys(store).length;
  },
  key: (index: number) => Object.keys(store)[index] ?? null,
};

vi.stubGlobal("localStorage", localStorageMock);

// Minimal CustomEvent stub (happy-dom provides it, but ensure it's available).
if (typeof globalThis.CustomEvent === "undefined") {
  class CE extends Event {
    detail: unknown;
    constructor(type: string, options?: CustomEventInit) {
      super(type, options);
      this.detail = options?.detail ?? null;
    }
  }
  vi.stubGlobal("CustomEvent", CE);
}
