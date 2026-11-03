import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import RecipientInput from "./RecipientInput.svelte";

describe("destinataires", () => {
  it("affiche toujours l'adresse réelle, jamais un nom qui imite une adresse", () => {
    render(RecipientInput, {
      label: "À",
      recipients: [
        { name: "Bob Dupont", address: "bob@example.com" },
        { name: "Alice Martin <alice@entreprise.fr>", address: "alice@evil.example" },
      ],
      suggestions: [],
      onAdd: vi.fn(() => true),
      onRemove: vi.fn(),
      onQuery: vi.fn(),
    });
    expect(screen.getByText("Bob Dupont <bob@example.com>")).toBeInTheDocument();
    // Nom usurpant une adresse : seule l'adresse réelle est montrée.
    expect(screen.getByText("alice@evil.example")).toBeInTheDocument();
    expect(screen.queryByText(/alice@entreprise\.fr/)).toBeNull();
  });
});
