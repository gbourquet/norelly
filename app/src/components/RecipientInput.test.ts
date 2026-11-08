import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import RecipientInput from "./RecipientInput.svelte";
import type { ContactListItemDto, RecipientSuggestion } from "../lib/ipc/api";

function setup(suggestions: RecipientSuggestion[] = [], lists: ContactListItemDto[] = []) {
  const onAdd = vi.fn(async () => true);
  const onAddList = vi.fn(async () => true);
  const onQuery = vi.fn();
  render(RecipientInput, {
    lists,
    onAddList,
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
  return { onAdd, onAddList, onQuery, input: screen.getByRole("combobox", { name: "À" }) };
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

  it("montre l'adresse d'une suggestion à part du nom, pour qu'un nom long ne la cache pas", async () => {
    const long = `Alice Martin - Direction financière ${"x".repeat(200)}`;
    const { input } = setup([{ name: long, address: "pirate@exemple.net", contact: null }]);
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "ali" } });
    const address = screen.getByText("<pirate@exemple.net>");
    expect(address).toHaveClass("address");
    expect(screen.getByText(long)).not.toContainElement(address);
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

  it("propose les listes du carnet en tête, avec leur nombre de membres et leur Cci", async () => {
    const { input } = setup(
      [{ name: "Fanny", address: "fanny@example.com", contact: null }],
      [
        { id: 5, name: "Famille", members: 4, bcc: true },
        { id: 6, name: "Fanfare", members: 1, bcc: false },
      ],
    );
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "fa" } });
    const [family, band, person] = screen.getAllByRole("option");
    expect(family).toHaveAccessibleName(/Famille.*4 membre\(s\).*Cci/);
    expect(band).toHaveAccessibleName(/Fanfare.*1 membre\(s\)/);
    expect(band).not.toHaveTextContent("Cci");
    expect(person).toHaveAccessibleName("Fanny <fanny@example.com>");
  });

  it("déplie la liste choisie à la souris ou au clavier, puis vide la saisie", async () => {
    const { input, onAddList, onAdd } = setup(
      [],
      [{ id: 5, name: "Famille", members: 2, bcc: false }],
    );
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "fam" } });
    await fireEvent.click(screen.getByRole("button", { name: /Famille/ }));
    expect(onAddList).toHaveBeenCalledWith(5);
    expect(onAdd).not.toHaveBeenCalled();
    await waitFor(() => expect(input).toHaveValue(""));
    await fireEvent.input(input, { target: { value: "fam" } });
    await fireEvent.keyDown(input, { key: "ArrowDown" });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onAddList).toHaveBeenCalledTimes(2);
  });
});
