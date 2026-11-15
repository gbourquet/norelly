import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "./paraglide/runtime.js";
import { fileSize, listedName, localUtcOffset, personName, shortDate } from "./format";

describe("mise en forme", () => {
  it("date courte selon l'ancienneté", () => {
    const now = new Date(2026, 10, 2, 15, 0);
    const today = new Date(2026, 10, 2, 9, 5).getTime() / 1000;
    expect(shortDate(today, now)).toBe("09:05");
    expect(shortDate(new Date(2026, 2, 12).getTime() / 1000, now)).toMatch(/12 mars/);
    expect(shortDate(new Date(2024, 2, 12).getTime() / 1000, now)).toBe("12/03/2024");
  });

  it("tailles et noms", () => {
    expect(fileSize(512)).toBe("512 o");
    expect(fileSize(2048)).toBe("2 Ko");
    expect(fileSize(3.5 * 1024 * 1024)).toBe("3,5 Mo");
    expect(personName({ name: "", address: "a@b.c" })).toBe("a@b.c");
    expect(personName(null)).toBe("(expéditeur inconnu)");
  });

  it("nom de liste : l'adresse aussi quand le domaine le demande", () => {
    const fake = { name: "Banque.fr", address: "pirate@exemple.net", show_address: true };
    // L'adresse d'abord, pour qu'un nom long ne la repousse pas hors de
    // vue ; chaque partie isolée (sens d'écriture) dans une liste.
    expect(listedName(fake)).toBe("⁨pirate@exemple.net⁩ (⁨Banque.fr⁩)");
    expect(listedName(fake, { isolate: false })).toBe("pirate@exemple.net (Banque.fr)");
    expect(personName(fake)).toBe("Banque.fr");
    expect(listedName({ name: "Bob", address: "bob@example.com", show_address: false })).toBe(
      "Bob",
    );
    expect(listedName({ name: "", address: "x@y.z", show_address: true })).toBe("x@y.z");
    expect(listedName(null)).toBe("(expéditeur inconnu)");
    // Une longue partie locale est abrégée : le domaine réel reste en vue.
    const long = {
      name: "Banque.fr",
      address: "banque.fr.service.client.securite.alerte@pirate.net",
      show_address: true,
    };
    expect(listedName(long, { isolate: false })).toBe(
      "banque.fr.service.cl…@pirate.net (Banque.fr)",
    );
  });
});

describe("mise en forme dans la locale active", () => {
  afterEach(() => setLocale("fr", { reload: false }));

  it("suit la langue de l'interface", () => {
    setLocale("en", { reload: false });
    const now = new Date(2026, 10, 2, 15, 0);
    expect(shortDate(new Date(2026, 2, 12).getTime() / 1000, now)).toMatch(/Mar 12/);
    expect(fileSize(512)).toBe("512 B");
    expect(fileSize(3.5 * 1024 * 1024)).toBe("3.5 MB");
    expect(personName(null)).toBe("(unknown sender)");
  });
});

describe("fuseau local", () => {
  it("donne le décalage UTC en minutes, de signe inverse à getTimezoneOffset", () => {
    const paris = { getTimezoneOffset: () => -60 } as Date;
    expect(localUtcOffset(paris)).toBe(60);
    const quebec = { getTimezoneOffset: () => 300 } as Date;
    expect(localUtcOffset(quebec)).toBe(-300);
  });
});
