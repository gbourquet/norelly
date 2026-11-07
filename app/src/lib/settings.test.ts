import { describe, expect, it, vi } from "vitest";
import type { SettingsDto } from "./ipc/api";
import { SettingsState, type SettingsApi } from "./settings.svelte";
import { IpcFailure } from "./errors";

const defaults: SettingsDto = {
  language: "system",
  update_check: "not_asked",
  notifications: true,
  notification_privacy: false,
  attachment_threshold: 2 * 1024 * 1024,
  mark_seen_delay_secs: 5,
};

function fakeApi(initial = defaults) {
  let stored = { ...initial };
  const api: SettingsApi = {
    getSettings: vi.fn(async () => ({ ...stored })),
    saveSettings: vi.fn(async (s: SettingsDto) => {
      if (s.attachment_threshold < 65536) throw new IpcFailure("invalid_input", "settings");
      stored = { ...s };
    }),
  };
  return api;
}

describe("paramètres", () => {
  it("pose la question des mises à jour au premier lancement, oui ou non", async () => {
    const api = fakeApi();
    const settings = new SettingsState(api);
    expect(settings.askUpdate).toBe(false);
    await settings.load();
    expect(settings.askUpdate).toBe(true);
    await settings.answerUpdate(true);
    expect(api.saveSettings).toHaveBeenLastCalledWith({ ...defaults, update_check: "enabled" });
    expect(settings.askUpdate).toBe(false);
    await settings.answerUpdate(false);
    expect(settings.current?.update_check).toBe("disabled");
  });

  it("enregistre, ou garde l'état et affiche l'erreur", async () => {
    const settings = new SettingsState(fakeApi({ ...defaults, update_check: "enabled" }));
    await settings.load();
    expect(settings.askUpdate).toBe(false);
    const next = { ...defaults, update_check: "enabled" as const, notification_privacy: true };
    expect(await settings.save(next)).toBe(true);
    expect(settings.current).toEqual(next);
    expect(await settings.save({ ...next, attachment_threshold: 1 })).toBe(false);
    expect(settings.current).toEqual(next);
    expect(settings.error).toContain("paramètres");
  });

  it("fermer la question sans répondre n'enregistre rien (reposée au lancement suivant)", async () => {
    const api = fakeApi();
    const settings = new SettingsState(api);
    await settings.load();
    expect(settings.askUpdate).toBe(true);
    settings.dismissUpdate();
    expect(settings.askUpdate).toBe(false);
    expect(api.saveSettings).not.toHaveBeenCalled();
    expect(settings.current?.update_check).toBe("not_asked");
  });
});
