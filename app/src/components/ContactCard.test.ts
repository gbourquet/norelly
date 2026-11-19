// Fiche d'un contact en lecture : titre et organisation, anniversaire,
// adresses postales et sites web ; un site s'ouvre par Rust. Photo servie
// par le protocole dédié, initiale à défaut (pas de photo, ou photo
// indisponible).
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
    photo: false,
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

function withPhoto(id: number, photo: boolean): ContactDto {
  return {
    id,
    name: "élodie Cézanne",
    photo,
    fields: {
      ...emptyFields(),
      display_name: "élodie Cézanne",
      emails: [{ address: "elodie@example.com", label: "", preferred: true }],
    },
  };
}

function card(c: ContactDto) {
  return render(ContactCard, {
    props: {
      contact: c,
      registry: new CommandRegistry<unknown>(),
      context: null,
      openWebsite: vi.fn(async () => undefined),
    },
  });
}

describe("photo d'un contact", () => {
  it("montre la photo du contact qui en a une", () => {
    const { container } = card(withPhoto(12, true));
    const img = container.querySelector(".avatar img");
    expect(img?.getAttribute("src")).toBe("contact-photo://localhost/12");
    // Décorative : le nom est déjà écrit à côté.
    expect(img?.getAttribute("alt")).toBe("");
  });

  it("montre l'initiale sans photo", () => {
    const { container } = card(withPhoto(12, false));
    expect(container.querySelector(".avatar img")).toBeNull();
    expect(container.querySelector(".avatar")?.textContent?.trim()).toBe("É");
  });

  it("revient à l'initiale si la photo ne se charge pas, jusqu'au contact suivant", async () => {
    const { container, rerender } = card(withPhoto(12, true));
    const img = container.querySelector(".avatar img");
    if (img === null) throw new Error("photo absente");
    await fireEvent.error(img);
    expect(container.querySelector(".avatar img")).toBeNull();
    expect(container.querySelector(".avatar")?.textContent?.trim()).toBe("É");
    await rerender({ contact: withPhoto(13, true) });
    expect(container.querySelector(".avatar img")?.getAttribute("src")).toBe(
      "contact-photo://localhost/13",
    );
  });
});
