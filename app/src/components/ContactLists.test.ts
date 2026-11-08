// Écrans des listes de diffusion : fiche d'une liste, fenêtre de création
// ou de modification, ajout de contacts à une liste, suppression.
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import ContactListCard from "./ContactListCard.svelte";
import ContactListDialog from "./ContactListDialog.svelte";
import AddToListDialog from "./AddToListDialog.svelte";
import DeleteContactListDialog from "./DeleteContactListDialog.svelte";
import { CommandRegistry } from "../lib/commands/registry";
import { IpcFailure } from "../lib/errors";
import {
  AddToListState,
  ContactListFormState,
  DeleteContactListState,
  type ContactListsApi,
} from "../lib/contact-lists.svelte";
import type { ContactItem, ContactList } from "../lib/ipc/api";

function person(id: number, name: string): ContactItem {
  return {
    id,
    name,
    address: `${name.toLowerCase()}@example.com`,
    organization: null,
    favorite: false,
  };
}

const ana = person(1, "Ana");
const bo = person(2, "Bo");

function list(fields: Partial<ContactList["fields"]> = {}, members: ContactItem[] = [ana, bo]) {
  return {
    id: 7,
    fields: {
      name: "Famille",
      description: "",
      members: members.map((m) => m.id),
      bcc: false,
      ...fields,
    },
    members,
  };
}

function fakeApi(): ContactListsApi {
  return {
    listContactLists: vi.fn(async () => ({
      items: [
        { id: 7, name: "Famille", members: 2, bcc: false },
        { id: 8, name: "Club", members: 0, bcc: false },
      ],
      next: null,
    })),
    getContactList: vi.fn(async () => list()),
    createContactList: vi.fn(async (fields) => ({ ...list(), fields })),
    updateContactList: vi.fn(async (_id, fields) => ({ ...list(), fields })),
    deleteContactList: vi.fn(async () => true),
    addToContactList: vi.fn(async (id) => ({ ...list(), id })),
    listContacts: vi.fn(async () => ({ items: [ana, bo], next: null })),
  };
}

describe("fiche d'une liste", () => {
  function registry() {
    const r = new CommandRegistry<unknown>();
    const commands: [string, string][] = [
      ["lists.write", "Écrire à la liste"],
      ["lists.edit", "Modifier la liste"],
      ["lists.delete", "Supprimer la liste"],
    ];
    for (const [id, title] of commands)
      r.register({ id, title, group: "Contacts", run: () => undefined });
    return r;
  }

  it("montre les membres à leur adresse préférée, la description en texte et le Cci", () => {
    render(ContactListCard, {
      props: {
        list: list({ description: "<b>Cousins</b>", bcc: true }),
        registry: registry(),
        context: null,
      },
    });
    expect(screen.getByRole("article", { name: "Liste Famille" })).toBeInTheDocument();
    expect(screen.getByText("2 membre(s)")).toBeInTheDocument();
    expect(screen.getByText("ana@example.com")).toBeInTheDocument();
    expect(screen.getByText("<b>Cousins</b>")).toBeInTheDocument();
    expect(screen.getByText(/copie cachée/)).toBeInTheDocument();
  });

  it("explique comment ajouter des membres à une liste vide", () => {
    render(ContactListCard, { props: { list: list({}, []), registry: registry(), context: null } });
    expect(screen.getByText(/Aucun membre pour l'instant/)).toBeInTheDocument();
    expect(screen.queryByText(/copie cachée/)).not.toBeInTheDocument();
  });
});

describe("fenêtre d'une liste", () => {
  it("cherche, ajoute et retire des membres, puis enregistre", async () => {
    const api = fakeApi();
    const form = new ContactListFormState(api);
    form.reset(null);
    const onSaved = vi.fn();
    render(ContactListDialog, { open: true, form, onSaved, onClose: vi.fn() });
    expect(screen.getByRole("button", { name: "Enregistrer" })).toBeDisabled();
    await fireEvent.input(screen.getByLabelText("Nom de la liste"), {
      target: { value: "Famille" },
    });
    await fireEvent.click(screen.getByRole("checkbox"));
    await fireEvent.input(screen.getByRole("searchbox", { name: "Ajouter un contact" }), {
      target: { value: "a" },
    });
    await fireEvent.click(await screen.findByRole("button", { name: "Ajouter Ana" }));
    expect(screen.getByText("Déjà membre")).toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "Ajouter Bo" }));
    expect(screen.getByText("Membres (2)")).toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "Retirer Ana de la liste" }));
    expect(screen.getByText("Membres (1)")).toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
    expect(api.createContactList).toHaveBeenCalledWith({
      name: "Famille",
      description: "",
      members: [2],
      bcc: true,
    });
  });

  it("garde la saisie et affiche le refus", async () => {
    const api = fakeApi();
    api.updateContactList = vi.fn(async () => {
      throw new IpcFailure("contact_list_duplicate_name", "list_name", "Club");
    });
    const form = new ContactListFormState(api);
    form.reset(list());
    const onSaved = vi.fn();
    render(ContactListDialog, { open: true, form, onSaved, onClose: vi.fn() });
    expect(screen.getByRole("dialog", { name: "Modifier la liste" })).toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "Enregistrer" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Une autre liste porte déjà ce nom");
    expect(onSaved).not.toHaveBeenCalled();
  });

  it("signale une recherche sans résultat", async () => {
    const api = fakeApi();
    api.listContacts = vi.fn(async () => ({ items: [], next: null }));
    const form = new ContactListFormState(api);
    form.reset(null);
    render(ContactListDialog, { open: true, form, onSaved: vi.fn(), onClose: vi.fn() });
    await fireEvent.input(screen.getByRole("searchbox", { name: "Ajouter un contact" }), {
      target: { value: "zz" },
    });
    expect(await screen.findByText("Aucun contact ne correspond.")).toBeInTheDocument();
  });
});

describe("ajout à une liste", () => {
  it("propose les listes, la première choisie, et ajoute à celle choisie", async () => {
    const api = fakeApi();
    const form = new AddToListState(api);
    await form.open();
    const onAdded = vi.fn();
    render(AddToListDialog, { open: true, ids: [1], name: "Ana", form, onAdded, onClose: vi.fn() });
    expect(screen.getByText("Ajouter « Ana » à la liste :")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /Famille/ })).toBeChecked();
    await fireEvent.click(screen.getByRole("radio", { name: /Club/ }));
    await fireEvent.click(screen.getByRole("button", { name: "Ajouter" }));
    await waitFor(() => expect(onAdded).toHaveBeenCalled());
    expect(api.addToContactList).toHaveBeenCalledWith(8, [1]);
  });

  it("sans liste, explique comment en créer une", async () => {
    const api = fakeApi();
    api.listContactLists = vi.fn(async () => ({ items: [], next: null }));
    const form = new AddToListState(api);
    await form.open();
    render(AddToListDialog, {
      open: true,
      ids: [1, 2],
      name: null,
      form,
      onAdded: vi.fn(),
      onClose: vi.fn(),
    });
    expect(screen.getByText("Ajouter 2 contacts à la liste :")).toBeInTheDocument();
    expect(screen.getByText(/Aucune liste pour l'instant/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Ajouter" })).toBeDisabled();
  });
});

describe("suppression d'une liste", () => {
  it("rappelle que les contacts restent, puis supprime", async () => {
    const api = fakeApi();
    const form = new DeleteContactListState(api);
    const onDeleted = vi.fn();
    render(DeleteContactListDialog, {
      open: true,
      id: 7,
      name: "Famille",
      form,
      onDeleted,
      onClose: vi.fn(),
    });
    expect(screen.getByText("Supprimer la liste « Famille » ?")).toBeInTheDocument();
    expect(screen.getByText(/restent dans le carnet/)).toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "Supprimer" }));
    await waitFor(() => expect(onDeleted).toHaveBeenCalledWith(7));
  });

  it("affiche l'échec sans fermer", async () => {
    const api = fakeApi();
    api.deleteContactList = vi.fn(async () => {
      throw new IpcFailure("contact_list_not_found");
    });
    const form = new DeleteContactListState(api);
    const onDeleted = vi.fn();
    render(DeleteContactListDialog, {
      open: true,
      id: 7,
      name: "Famille",
      form,
      onDeleted,
      onClose: vi.fn(),
    });
    await fireEvent.click(screen.getByRole("button", { name: "Supprimer" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Cette liste n'existe plus.");
    expect(onDeleted).not.toHaveBeenCalled();
  });
});
