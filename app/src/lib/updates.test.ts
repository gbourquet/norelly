import { describe, expect, it, vi } from "vitest";
import { UpdatesState, type UpdatesApi } from "./updates.svelte";
import { IpcFailure } from "./errors";

function fakeApi(supported: boolean, available: string | null = null) {
  const api: UpdatesApi = {
    updateStatus: vi.fn(async () => ({ supported, available })),
    installUpdate: vi.fn(async () => undefined),
  };
  return api;
}

describe("mises à jour", () => {
  it("rien n'est proposé sur un canal qui ne se met pas à jour lui-même", async () => {
    const updates = new UpdatesState(fakeApi(false));
    expect(updates.supported).toBe(false);
    await updates.load();
    expect(updates.supported).toBe(false);
    expect(updates.available).toBeNull();
  });

  it("une mise à jour trouvée (au lancement ou plus tard) est annoncée", async () => {
    const updates = new UpdatesState(fakeApi(true, "1.1.0"));
    await updates.load();
    expect(updates.supported).toBe(true);
    expect(updates.available).toBe("1.1.0");
    updates.handle({ kind: "available", version: "1.2.0" });
    expect(updates.available).toBe("1.2.0");
  });

  it("installe, ou affiche l'erreur et permet de réessayer", async () => {
    const api = fakeApi(true, "1.1.0");
    vi.mocked(api.installUpdate).mockRejectedValueOnce(
      new IpcFailure("update_failed", null, "signature invalide"),
    );
    const updates = new UpdatesState(api);
    await updates.load();
    await updates.install();
    expect(updates.error).toContain("signature invalide");
    expect(updates.installing).toBe(false);
    await updates.install();
    expect(api.installUpdate).toHaveBeenCalledTimes(2);
    expect(updates.error).toBeNull();
  });

  it("le bandeau se ferme pour la session", async () => {
    const updates = new UpdatesState(fakeApi(true, "1.1.0"));
    await updates.load();
    updates.dismiss();
    expect(updates.available).toBeNull();
  });
});
