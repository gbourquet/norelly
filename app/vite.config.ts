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
  },
});
