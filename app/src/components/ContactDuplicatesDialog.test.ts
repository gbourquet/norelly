import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import ContactDuplicatesDialog from "./ContactDuplicatesDialog.svelte";
import { ContactDuplicatesState } from "../lib/contact-duplicates.svelte";
import { IpcFailure } from "../lib/errors";
import type { ContactDuplicateGroupDto, DuplicateCardDto } from "../lib/ipc/api";

function card(id: number, name: string, extra: Partial<DuplicateCardDto> = {}): DuplicateCardDto {
  return {
    id,
    name,
    emails: [{ address: `c${id}@example.com`, label: "", preferred: true }],
    phones: [],
    organization: "",
    ...extra,
  };
}

const group: ContactDuplicateGroupDto = {
  contacts: [
    card(3, "Alice Martin", { phones: ["06 12 34 56 78"], organization: "ACME" }),
    card(1, "Martin Alice"),
  ],
  reasons: ["same_name", "same_phone"],
  truncated: true,
};

function api(groups: ContactDuplicateGroupDto[], merge?: Error) {
  return {
    findContactDuplicates: vi.fn(async () => groups),
    previewContactMerge: vi.fn(async (keep: number) => ({
      name: "Alice Martin",
      preferred_address: `c${keep}@example.com`,
    })),
    mergeContacts: vi.fn(async (keep: number) => {
      if (merge) throw merge;
      return {
        id: keep,
        name: "Alice Martin",
        fields: {
          display_name: "",
          given_name: "",
          family_name: "",
          nickname: "",
          organization: "",
          notes: "",
          emails: [],
          phones: [],
          favorite: false,
        },
      };
    }),
  };
}

function show(form: ContactDuplicatesState) {
  const onMerged = vi.fn();
  const onClose = vi.fn();
  render(ContactDuplicatesDialog, { open: true, form, onMerged, onClose });
  return { onMerged, onClose };
}

describe("fenêtre des doublons", () => {
  it("présente le groupe : raisons, fiches, aperçu sous la fiche gardée, groupe tronqué", async () => {
    const form = new ContactDuplicatesState(api([group]));
    await form.open();
    show(form);
    expect(screen.getByText("Groupe 1 sur 1")).toBeTruthy();
    expect(screen.getByText("Même nom")).toBeTruthy();
    expect(screen.getByText("Même téléphone")).toBeTruthy();
    expect(screen.getByText("06 12 34 56 78")).toBeTruthy();
    expect(screen.getByText("ACME")).toBeTruthy();
    expect(screen.getByText(/seules les 20 plus complètes/)).toBeTruthy();
    expect(
      screen.getByText("Après la fusion : Alice Martin, adresse préférée c3@example.com"),
    ).toBeTruthy();
    const kept = screen.getByLabelText("Réunir Alice Martin") as HTMLInputElement;
    expect(kept.checked && kept.disabled).toBe(true);
  });

  it("fusionne, prévient l'application, puis annonce la fin de l'examen", async () => {
    const form = new ContactDuplicatesState(api([group]));
    await form.open();
    const { onMerged } = show(form);
    await fireEvent.click(screen.getByRole("button", { name: "Fusionner" }));
    await vi.waitFor(() => expect(onMerged).toHaveBeenCalled());
    expect(await screen.findByText("Tous les doublons ont été examinés.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Fusionner" })).toBeNull();
  });

  it("garder une autre fiche, décocher, passer, fermer", async () => {
    const form = new ContactDuplicatesState(api([group]));
    await form.open();
    const { onClose } = show(form);
    const radios = screen.getAllByRole("radio");
    await fireEvent.click(radios[1] as HTMLElement);
    await vi.waitFor(() => expect(form.keep).toBe(1));
    await fireEvent.click(screen.getByLabelText("Réunir Alice Martin"));
    await vi.waitFor(() => expect(form.isChecked(3)).toBe(false));
    expect((screen.getByRole("button", { name: "Fusionner" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    await fireEvent.click(screen.getByRole("button", { name: "Passer" }));
    expect(await screen.findByText("Tous les doublons ont été examinés.")).toBeTruthy();
    await fireEvent.click(screen.getByText("Fermer"));
    expect(onClose).toHaveBeenCalled();
  });

  it("sans doublon : message", async () => {
    const empty = new ContactDuplicatesState(api([]));
    await empty.open();
    show(empty);
    expect(screen.getByText(/Aucun doublon trouvé/)).toBeTruthy();
  });

  it("une fusion refusée affiche son message et garde le groupe", async () => {
    const form = new ContactDuplicatesState(api([group], new IpcFailure("contact_merge_conflict")));
    await form.open();
    show(form);
    await fireEvent.click(screen.getByRole("button", { name: "Fusionner" }));
    expect((await screen.findByRole("alert")).textContent).toContain("modifiée entre-temps");
    expect(screen.getByText("Groupe 1 sur 1")).toBeTruthy();
  });

  it("recherche en cours : message d'attente", () => {
    const form = new ContactDuplicatesState(api([group]));
    void form.open();
    show(form);
    expect(screen.getByText("Recherche des doublons…")).toBeTruthy();
  });
});
