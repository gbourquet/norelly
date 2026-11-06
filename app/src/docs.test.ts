// Documentation utilisateur : chaque commande du registre figure
// dans l'aide-mémoire des raccourcis, dans les deux langues, avec son
// libellé de l'interface et, si elle en a un, son raccourci tel que l'aide
// (« ? ») l'affiche. Lancé aussi par scripts/check-docs.sh.
import { readFileSync } from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { registerAppCommands, registry } from "./lib/app.svelte";
import { formatShortcut } from "./lib/commands/shortcut";
import { setLocale } from "./lib/paraglide/runtime.js";

vi.mock("./lib/ipc/api", () => ({ IpcFailure: class extends Error {}, api: {} }));

const docs = path.resolve(__dirname, "../../docs/utilisateur");

/** Lignes de l'aide-mémoire d'une langue. */
function cheatSheet(lang: "fr" | "en"): string[] {
  return readFileSync(path.join(docs, lang, "shortcuts.md"), "utf8").split("\n");
}

describe("aide-mémoire des raccourcis", () => {
  let off: (() => void) | null = null;
  afterEach(() => {
    off?.();
    off = null;
    setLocale("fr", { reload: false });
  });

  for (const lang of ["fr", "en"] as const) {
    it(`contient chaque commande du registre (${lang})`, () => {
      setLocale(lang, { reload: false });
      off = registerAppCommands();
      const lines = cheatSheet(lang);
      const missing: string[] = [];
      for (const command of registry.all()) {
        const keys = command.parsedShortcut
          ? `\`${formatShortcut(command.parsedShortcut).join("+")}\``
          : null;
        const row = lines.find((l) => l.startsWith("|") && l.includes(`| ${command.title} |`));
        if (row === undefined) missing.push(`${command.id} : ligne « ${command.title} » absente`);
        else if (keys !== null && !row.includes(keys))
          missing.push(`${command.id} : raccourci ${keys} absent de la ligne « ${command.title} »`);
      }
      expect(registry.all().length).toBeGreaterThan(20);
      expect(missing).toEqual([]);
    });
  }
});
