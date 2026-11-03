/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { paraglideVitePlugin } from "@inlang/paraglide-js";

// Configuration adaptée à Tauri : port fixe, pas d'écran d'erreur masquant les logs Rust.
export default defineConfig({
  plugins: [
    // Catalogues de traduction compilés (ADR-011) ; mêmes options que `pnpm i18n`.
    paraglideVitePlugin({
      project: "./project.inlang",
      outdir: "./src/lib/paraglide",
      strategy: ["globalVariable", "baseLocale"],
      emitTsDeclarations: true,
    }),
    svelte(),
  ],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2022",
    outDir: "dist",
  },
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    setupFiles: ["./vitest-setup.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,svelte}"],
      // Code généré (catalogues, bindings IPC), point d'entrée et types.
      exclude: ["src/lib/paraglide/**", "src/lib/ipc/bindings.ts", "src/main.ts", "src/**/*.d.ts"],
      reporter: ["text-summary", "text"],
      // Seuils vérifiés par ./scripts/check.sh (mesure du 2026-11-03) : à
      // relever, jamais à baisser.
      thresholds: { lines: 78, statements: 74, branches: 67, functions: 72 },
    },
  },
});
