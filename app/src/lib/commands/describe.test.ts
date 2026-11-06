import { describe, expect, it } from "vitest";
import { CommandRegistry } from "./registry";
import { describeCommand } from "./describe";

interface Ctx {
  open: boolean;
}

function registry(): CommandRegistry<Ctx> {
  const r = new CommandRegistry<Ctx>();
  r.register({ id: "a.archive", title: "Archiver", group: "G", shortcut: "e", run: () => {} });
  r.register({
    id: "a.read",
    title: "Marquer comme lu",
    group: "G",
    shortcut: "Shift+I",
    when: (ctx) => ctx.open,
    run: () => {},
  });
  r.register({ id: "a.sync", title: "Synchroniser", group: "G", run: () => {} });
  return r;
}

describe("describeCommand", () => {
  it("donne le libellé, les touches et l'infobulle « Libellé — Touche »", () => {
    const d = describeCommand(registry(), "a.archive", { open: false });
    expect(d).toEqual({
      label: "Archiver",
      keys: "E",
      tooltip: "Archiver — E",
      enabled: true,
    });
  });

  it("affiche les modificateurs dans les touches", () => {
    const d = describeCommand(registry(), "a.read", { open: true });
    expect(d?.keys).toBe("Maj+I");
    expect(d?.tooltip).toBe("Marquer comme lu — Maj+I");
  });

  it("sans raccourci, l'infobulle est le libellé seul", () => {
    const d = describeCommand(registry(), "a.sync", { open: false });
    expect(d?.keys).toBe("");
    expect(d?.tooltip).toBe("Synchroniser");
  });

  it("reflète la condition d'activation", () => {
    expect(describeCommand(registry(), "a.read", { open: false })?.enabled).toBe(false);
  });

  it("un libellé de remplacement garde le raccourci de la commande", () => {
    const d = describeCommand(registry(), "a.archive", { open: false }, "Ranger");
    expect(d?.label).toBe("Ranger");
    expect(d?.tooltip).toBe("Ranger — E");
  });

  it("commande inconnue : rien", () => {
    expect(describeCommand(registry(), "inconnue", { open: false })).toBeNull();
  });
});
