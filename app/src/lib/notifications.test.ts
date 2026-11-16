import { describe, expect, it } from "vitest";
import { notificationText, type NewMailEvent } from "./notifications";

const bob = {
  name: "Bob",
  address: "bob@example.com",
  contact: null,
  show_address: false,
  auth: "unknown",
  contact_badge: false,
};

function event(count: number, from: NewMailEvent["from"], subject: string | null): NewMailEvent {
  return { kind: "new_mail", account: 1, count, from, subject };
}

describe("texte des notifications", () => {
  it("un message : l'expéditeur en titre, le sujet en texte", () => {
    expect(notificationText(event(1, bob, "Devis"), "moi@example.com")).toEqual({
      title: "Bob",
      body: "Devis",
    });
    expect(
      notificationText(
        event(
          1,
          {
            name: "",
            address: "x@y.z",
            contact: null,
            show_address: false,
            auth: "unknown",
            contact_badge: false,
          },
          "",
        ),
        "moi@example.com",
      ),
    ).toEqual({
      title: "x@y.z",
      body: "(sans objet)",
    });
  });

  it("un nom en forme de domaine : l'adresse aussi", () => {
    const fake = {
      name: "Banque.fr",
      address: "pirate@exemple.net",
      contact: null,
      show_address: true,
      auth: "unknown",
      contact_badge: false,
    };
    expect(notificationText(event(1, fake, "Alerte"), "moi@example.com").title).toBe(
      "pirate@exemple.net (Banque.fr)",
    );
    expect(notificationText(event(2, fake, "Alerte"), "moi@example.com").body).toBe(
      "Dernier : pirate@exemple.net (Banque.fr) — Alerte",
    );
  });

  it("plusieurs messages : regroupés, avec le plus récent", () => {
    expect(notificationText(event(3, bob, "Devis"), "moi@example.com")).toEqual({
      title: "3 nouveaux messages",
      body: "Dernier : Bob — Devis",
    });
  });

  it("confidentialité : ni expéditeur ni sujet, seulement le compte", () => {
    expect(notificationText(event(1, null, null), "moi@example.com")).toEqual({
      title: "Nouveau message",
      body: "moi@example.com",
    });
    expect(notificationText(event(4, null, null), "moi@example.com")).toEqual({
      title: "4 nouveaux messages",
      body: "moi@example.com",
    });
  });
});
