import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import RecipientInput from "./RecipientInput.svelte";
import type { RecipientSuggestion } from "../lib/ipc/api";

function setup(suggestions: RecipientSuggestion[] = []) {
  const onAdd = vi.fn(async () => true);
  const onQuery = vi.fn();
  render(RecipientInput, {
    label: "À",
    recipients: [
      { name: "Bob Dupont", address: "bob@example.com" },
      { name: "", address: "alice@evil.example" },
    ],
    suggestions,
    onAdd,
    onRemove: vi.fn(),
    onQuery,
  });
  return { onAdd, onQuery, input: screen.getByRole("combobox", { name: "À" }) };
}

describe("destinataires", () => {
  it("affiche chaque puce telle que lue par Rust, adresse réelle en infobulle", () => {
    // Le nom d'une puce vient toujours de Rust (Recipient::parse, brouillon
    // relu), qui écarte un nom imitant une adresse : sans nom, l'adresse.
    setup();
    expect(screen.getByText("Bob Dupont <bob@example.com>")).toHaveAttribute(
      "title",
      "bob@example.com",
    );
    expect(screen.getByText("alice@evil.example")).toBeInTheDocument();
  });

  it("distingue un contact du carnet d'une adresse déjà utilisée", async () => {
    const { input } = setup([
      { name: "Alice Martin", address: "alice@example.com", contact: 3 },
      { name: "Alicia", address: "alicia@example.com", contact: null },
    ]);
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "ali" } });
    const [contact, collected] = screen.getAllByRole("option");
    expect(contact).toHaveAccessibleName(/Alice Martin <alice@example\.com>.*Dans vos contacts/);
    expect(collected).toHaveAccessibleName("Alicia <alicia@example.com>");
    expect(collected).not.toHaveTextContent("Dans vos contacts");
  });

  it("ajoute la suggestion choisie et vide la saisie une fois acceptée", async () => {
    const { input, onAdd, onQuery } = setup([
      { name: "Alice Martin", address: "alice@example.com", contact: 3 },
    ]);
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "ali" } });
    await fireEvent.click(screen.getByRole("button", { name: /Alice Martin/ }));
    expect(onAdd).toHaveBeenCalledWith("Alice Martin <alice@example.com>");
    await waitFor(() => expect(input).toHaveValue(""));
    expect(onQuery).toHaveBeenLastCalledWith("");
  });
});
