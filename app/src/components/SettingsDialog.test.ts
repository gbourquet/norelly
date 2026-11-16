import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import SettingsDialog from "./SettingsDialog.svelte";
import type { SettingsDto } from "../lib/ipc/api";

const settings: SettingsDto = {
  language: "system",
  update_check: "enabled",
  notifications: true,
  notification_privacy: false,
  attachment_threshold: 2 * 1024 * 1024,
  mark_seen_delay_secs: 5,
};

function open(current: SettingsDto) {
  const onSave = vi.fn();
  render(SettingsDialog, {
    open: true,
    settings: current,
    plainText: false,
    updatesSupported: false,
    error: null,
    onSave,
    onClose: vi.fn(),
  });
  const select = screen.getByRole("combobox", {
    name: "Marquer comme lu un message ouvert",
  }) as HTMLSelectElement;
  return { onSave, select };
}

describe("paramètres : marquage comme lu", () => {
  it("propose immédiatement, quelques secondes ou jamais", () => {
    const { select } = open(settings);
    expect([...select.options].map((o) => o.textContent?.trim())).toEqual([
      "Immédiatement",
      "Après 2 s",
      "Après 5 s",
      "Après 10 s",
      "Après 30 s",
      "Jamais",
    ]);
    expect(select.selectedOptions[0]?.textContent?.trim()).toBe("Après 5 s");
  });

  it("enregistre « jamais » comme une absence de délai", async () => {
    const { onSave, select } = open(settings);
    await fireEvent.change(select, { target: { selectedIndex: 5 } });
    await fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    expect(onSave).toHaveBeenCalledWith({ ...settings, mark_seen_delay_secs: null }, false);
  });

  it("affiche le choix « jamais » enregistré", () => {
    const { select } = open({ ...settings, mark_seen_delay_secs: null });
    expect(select.selectedOptions[0]?.textContent?.trim()).toBe("Jamais");
  });

  it("garde affiché un délai enregistré hors de la liste", () => {
    const { select } = open({ ...settings, mark_seen_delay_secs: 12 });
    expect(select.selectedOptions[0]?.textContent?.trim()).toBe("Après 12 s");
  });
});
