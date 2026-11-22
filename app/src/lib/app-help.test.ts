// Aide de l'application : « Guide d'utilisation » et « Site de Norelly »
// ouvrent les pages du site dans la langue de l'interface. L'interface ne
// choisit qu'une page et une langue : l'adresse est construite en Rust.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setLocale } from "./paraglide/runtime.js";

const openSitePage = vi.fn<(page: string, language: string) => Promise<void>>(
  async () => undefined,
);

vi.mock("./ipc/api", () => ({
  api: {
    listConversations: vi.fn(async () => ({ items: [], next: null })),
    openSitePage: (page: string, language: string) => openSitePage(page, language),
  },
}));

const { app, mail, registerAppCommands, registry } = await import("./app.svelte");
const { IpcFailure } = await import("./errors");
const { m } = await import("./paraglide/messages.js");

describe("aide : pages du site", () => {
  let off: () => void = () => undefined;

  beforeEach(() => {
    off = registerAppCommands();
    openSitePage.mockClear();
    mail.notice = null;
  });

  afterEach(() => {
    off();
    setLocale("fr", { reload: false });
  });

  it("« Guide d'utilisation » ouvre le guide dans la langue de l'interface", async () => {
    setLocale("fr", { reload: false });
    expect(await registry.execute("help.guide", app.context())).toBe(true);
    setLocale("en", { reload: false });
    expect(await registry.execute("help.guide", app.context())).toBe(true);
    expect(openSitePage.mock.calls).toEqual([
      ["guide", "fr"],
      ["guide", "en"],
    ]);
  });

  it("« Site de Norelly » ouvre l'accueil du site dans la langue de l'interface", async () => {
    setLocale("en", { reload: false });
    expect(await registry.execute("help.website", app.context())).toBe(true);
    expect(openSitePage.mock.calls).toEqual([["home", "en"]]);
  });

  it("un échec d'ouverture est signalé, sans lever d'erreur", async () => {
    openSitePage.mockRejectedValueOnce(new IpcFailure("open_failed"));
    expect(await registry.execute("help.guide", app.context())).toBe(true);
    expect(mail.notice?.text).toBe(m.err_open_failed());
  });
});
