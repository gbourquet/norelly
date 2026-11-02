import js from "@eslint/js";
import ts from "typescript-eslint";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import svelteConfig from "./svelte.config.js";

export default ts.config(
  {
    ignores: [
      "dist/",
      "src-tauri/",
      "node_modules/",
      "src/lib/ipc/bindings.ts",
      "src/lib/paraglide/",
      "project.inlang/",
    ],
  },
  js.configs.recommended,
  ...ts.configs.strict,
  ...svelte.configs.recommended,
  {
    languageOptions: { globals: { ...globals.browser } },
  },
  {
    // Tests end-to-end : Node.js et Mocha (WebdriverIO).
    files: ["e2e/**/*.mjs"],
    languageOptions: { globals: { ...globals.node, ...globals.mocha } },
  },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: {
      parserOptions: {
        parser: ts.parser,
        extraFileExtensions: [".svelte"],
        svelteConfig,
      },
    },
  },
  {
    // Sécurité : jamais d'injection HTML brute dans l'interface (le HTML des
    // mails passe exclusivement par l'iframe mailview://, voir ADR-004).
    rules: {
      "svelte/no-at-html-tags": "error",
      "no-restricted-syntax": [
        "error",
        {
          selector: "AssignmentExpression[left.property.name='innerHTML']",
          message: "innerHTML interdit (ADR-004).",
        },
        { selector: "CallExpression[callee.name='eval']", message: "eval interdit." },
      ],
    },
  },
);
