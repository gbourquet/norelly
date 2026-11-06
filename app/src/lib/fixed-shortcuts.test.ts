import { describe, expect, it } from "vitest";
import { composeWindowShortcuts, formatTooltip } from "./fixed-shortcuts";

describe("raccourcis hors registre", () => {
  it("infobulle d'un bouton de mise en forme : « Libellé — Touches »", () => {
    expect(formatTooltip("bold")).toBe("Gras — Ctrl+B");
    expect(formatTooltip("strike")).toBe("Barré — Ctrl+Maj+S");
    expect(formatTooltip("bullet")).toBe("Liste à puces — Ctrl+Maj+8");
    // Sans raccourci : le libellé seul.
    expect(formatTooltip("link")).toBe("Lien");
  });

  it("fenêtre de rédaction : envoi, fermeture, puis mise en forme", () => {
    expect(composeWindowShortcuts().map((s) => [s.label, s.keys.join("+")])).toEqual([
      ["Envoyer le message", "Ctrl+Entrée"],
      ["Fermer en gardant le brouillon", "Échap"],
      ["Gras", "Ctrl+B"],
      ["Italique", "Ctrl+I"],
      ["Barré", "Ctrl+Maj+S"],
      ["Liste à puces", "Ctrl+Maj+8"],
      ["Liste numérotée", "Ctrl+Maj+7"],
      ["Citation", "Ctrl+Maj+B"],
    ]);
  });
});
