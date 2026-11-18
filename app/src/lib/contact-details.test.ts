import { describe, expect, it } from "vitest";
import {
  birthdayParts,
  birthdayText,
  composeBirthday,
  monthNames,
  postalLines,
} from "./contact-details";

describe("anniversaire d'une fiche", () => {
  it("se découpe en jour, mois et année facultative", () => {
    expect(birthdayParts("1985-04-12")).toEqual({ day: "12", month: "4", year: "1985" });
    expect(birthdayParts("--02-29")).toEqual({ day: "29", month: "2", year: "" });
    expect(birthdayParts("")).toEqual({ day: "", month: "", year: "" });
    expect(birthdayParts("n'importe quoi")).toEqual({ day: "", month: "", year: "" });
  });

  it("se recompose dans la forme de Rust ; une saisie incomplète part telle quelle, Rust la refuse", () => {
    expect(composeBirthday({ day: "12", month: "4", year: "1985" })).toBe("1985-04-12");
    expect(composeBirthday({ day: " 9 ", month: "12", year: "" })).toBe("--12-09");
    expect(composeBirthday({ day: "", month: "", year: "" })).toBe("");
    expect(composeBirthday({ day: "", month: "", year: "  " })).toBe("");
    expect(composeBirthday({ day: "12", month: "", year: "" })).toBe("--00-12");
    expect(composeBirthday({ day: "", month: "", year: "1985" })).toBe("1985-00-00");
    // Année à deux chiffres : envoyée telle quelle, Rust la refuse (jamais
    // complétée en une année de l'Antiquité).
    expect(composeBirthday({ day: "12", month: "4", year: "85" })).toBe("85-04-12");
  });

  it("s'affiche en toutes lettres, sans année si elle manque", () => {
    expect(birthdayText("1985-04-12", "fr")).toBe("12 avril 1985");
    expect(birthdayText("--04-12", "fr")).toBe("12 avril");
    expect(birthdayText("--02-29", "en")).toBe("February 29");
    expect(birthdayText("1985-04-12", "en")).toBe("April 12, 1985");
    // Forme inattendue : montrée telle quelle.
    expect(birthdayText("bizarre", "fr")).toBe("bizarre");
  });

  it("propose les douze mois dans la langue de l'interface", () => {
    const fr = monthNames("fr");
    expect(fr).toHaveLength(12);
    expect(fr[0]).toBe("janvier");
    expect(monthNames("en")[11]).toBe("December");
  });
});

describe("adresse postale d'une fiche", () => {
  it("s'affiche ligne à ligne, code postal et ville ensemble, éléments vides omis", () => {
    expect(
      postalLines({
        street: "1 rue des Lilas\nBât. B",
        locality: "Lyon",
        region: "Rhône",
        postal_code: "69001",
        country: "France",
        label: "",
      }),
    ).toEqual(["1 rue des Lilas", "Bât. B", "69001 Lyon", "Rhône", "France"]);
    expect(
      postalLines({
        street: "",
        locality: "",
        region: "Bretagne",
        postal_code: "",
        country: "",
        label: "maison",
      }),
    ).toEqual(["Bretagne"]);
  });
});
