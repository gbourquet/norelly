// Fiche d'un contact en lecture : titre et organisation, anniversaire,
// adresses postales et sites web ; un site s'ouvre par Rust.
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import ContactCard from "./ContactCard.svelte";
import { CommandRegistry } from "../lib/commands/registry";
import { IpcFailure } from "../lib/errors";
import { emptyFields } from "../lib/contacts.svelte";
import type { ContactDto } from "../lib/ipc/api";

function contact(): ContactDto {
  return {
    id: 3,
    name: "Hélène Lefèvre",
    fields: {
      ...emptyFields(),
      display_name: "Hélène Lefèvre",
      organization: "Mairie",
      title: "Adjointe",
      birthday: "1985-04-12",
      emails: [{ address: "helene@example.fr", label: "", preferred: true }],
      postal_addresses: [
        {
          street: "1 place de l'Hôtel-de-Ville\nBureau 3",
          locality: "Saint-Étienne",
          region: "",
          postal_code: "42000",
          country: "France",
          label: "work",
          label_kind: "work",
        },
      ],
      websites: [{ url: "https://mairie.example/elus", label: "", label_kind: null }],
    },
  };
}

function show(openWebsite: (url: string) => Promise<void>) {
  return render(ContactCard, {
    props: {
      contact: contact(),
      registry: new CommandRegistry<unknown>(),
      context: null,
      openWebsite,
    },
  });
}

describe("fiche d'un contact", () => {
  it("affiche fonction, organisation, anniversaire et adresse postale", () => {
    show(vi.fn(async () => undefined));
    expect(screen.getByText("Adjointe")).toBeTruthy();
    expect(screen.getByText("Mairie")).toBeTruthy();
    expect(screen.getByText("12 avril 1985")).toBeTruthy();
    const place = screen.getByRole("region", { name: "Adresses postales" });
    expect(place.textContent).toContain("1 place de l'Hôtel-de-Ville");
    expect(place.textContent).toContain("Bureau 3");
    expect(place.textContent).toContain("42000 Saint-Étienne");
    expect(place.textContent).toContain("Travail");
  });

  it("ouvre un site par Rust, son adresse entière sur le bouton", async () => {
    const open = vi.fn(async () => undefined);
    show(open);
    const button = screen.getByRole("button", { name: /https:\/\/mairie\.example\/elus/ });
    await fireEvent.click(button);
    expect(open).toHaveBeenCalledWith("https://mairie.example/elus");
  });

  it("explique un site que Rust refuse d'ouvrir", async () => {
    show(vi.fn(async () => Promise.reject(new IpcFailure("link_refused"))));
    await fireEvent.click(screen.getByRole("button", { name: /mairie\.example/ }));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Site non ouvert"));
  });
});
