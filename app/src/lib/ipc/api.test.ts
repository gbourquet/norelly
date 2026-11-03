import { describe, expect, it, vi } from "vitest";

vi.mock("./bindings", () => ({
  commands: {
    listAccounts: vi.fn(async () => ({ status: "ok", data: [{ id: 1 }] })),
    removeAccount: vi.fn(async () => ({
      status: "error",
      error: { code: "not_found", field: "account", detail: "absent" },
    })),
    outbox: vi.fn(async () => ({
      status: "error",
      error: { code: "storage", field: null, detail: null },
    })),
  },
  events: {},
}));

const { api, IpcFailure } = await import("./api");

describe("enveloppe des commandes IPC", () => {
  it("renvoie les données d'un succès", async () => {
    await expect(api.listAccounts()).resolves.toEqual([{ id: 1 }]);
  });

  it("transforme une erreur en IpcFailure structurée", async () => {
    const failure = await api.removeAccount(1).catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(IpcFailure);
    expect(failure).toMatchObject({ code: "not_found", field: "account", detail: "absent" });
    const storage = await api.outbox(1).catch((e: unknown) => e);
    expect(storage).toMatchObject({ code: "storage", field: null, detail: null });
  });
});
