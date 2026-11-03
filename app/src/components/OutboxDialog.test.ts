import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import OutboxDialog from "./OutboxDialog.svelte";
import type { OutboxItemDto } from "../lib/ipc/api";

const item = (id: number, state: OutboxItemDto["state"]): OutboxItemDto => ({
  id,
  subject: id === 3 ? "" : `Sujet ${id}`,
  recipients: ["bob@example.com"],
  state,
  attempts: 1,
  queued_at: 0,
});

describe("boîte d'envoi", () => {
  it("propose de réessayer un échec et d'annuler un envoi en attente", async () => {
    const onRetry = vi.fn();
    const onCancel = vi.fn();
    render(OutboxDialog, {
      open: true,
      items: [
        item(1, { state: "failed", failure: "rejected" }),
        item(2, { state: "queued", not_before: 0 }),
        item(3, { state: "sending" }),
      ],
      onRetry,
      onCancel,
      onClose: vi.fn(),
    });
    expect(screen.getByText("refusé par le serveur")).toBeInTheDocument();
    expect(screen.getByText("(sans objet)")).toBeInTheDocument();
    const retry = screen.getAllByRole("button", { name: "Réessayer" });
    expect(retry).toHaveLength(1);
    await fireEvent.click(retry[0] as HTMLElement);
    expect(onRetry).toHaveBeenCalledWith(1);
    const cancel = screen.getAllByRole("button", { name: "Annuler l'envoi" });
    // Rien pour un envoi en cours.
    expect(cancel).toHaveLength(2);
    await fireEvent.click(cancel[1] as HTMLElement);
    expect(onCancel).toHaveBeenCalledWith(2);
  });

  it("indique une file vide", () => {
    render(OutboxDialog, {
      open: true,
      items: [],
      onRetry: vi.fn(),
      onCancel: vi.fn(),
      onClose: vi.fn(),
    });
    expect(screen.getByText("Aucun message en attente.")).toBeInTheDocument();
  });
});
