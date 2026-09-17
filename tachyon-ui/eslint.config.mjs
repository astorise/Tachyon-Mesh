// Flat config (ESLint 9+). `@typescript-eslint/parser` is used only to let
// ESLint read `.ts` syntax; no `@typescript-eslint/eslint-plugin` type-aware
// rules are enabled, so it needs no `parserOptions.project` and no type
// information — see scripts/link-eslint-ts6.mjs for why the parser needs a
// TypeScript 6 side-by-side install to run at all against this project's
// TypeScript 7.
import tsParser from "@typescript-eslint/parser";

export default [
  {
    // Test fixtures build DOM structure via innerHTML too, but never with
    // attacker-reachable data — they set up known-static markup for
    // assertions, not render app UI. Excluded so this guard stays focused
    // on shipped code.
    ignores: ["**/*.test.ts"],
  },
  {
    files: ["**/*.ts"],
    languageOptions: {
      parser: tsParser,
      ecmaVersion: 2022,
      sourceType: "module",
    },
    rules: {
      "no-restricted-properties": [
        "error",
        {
          property: "innerHTML",
          message:
            "Direct innerHTML assignment risks XSS. Use textContent for plain text, replaceChildren() with DOM nodes for structured content, or ensure all interpolated values pass through escapeHtml() first.",
        },
        {
          property: "outerHTML",
          message:
            "Direct outerHTML assignment risks XSS. Use DOM manipulation methods instead.",
        },
      ],
    },
  },
];
