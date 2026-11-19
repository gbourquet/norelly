// Détails d'une fiche contact (anniversaire, adresse postale) : découpage
// pour le formulaire et mise en forme pour l'affichage. La validation est
// faite par Rust (ADR-012) : une saisie incomplète part telle quelle.

import type { ContactPostalAddressDto } from "./ipc/api";

/** Anniversaire saisi : jour, mois (1 à 12) et année facultative, en texte. */
export interface BirthdayParts {
  day: string;
  month: string;
  year: string;
}

/** Anniversaire de Rust : `1985-04-12`, ou `--04-12` sans année. */
const BIRTHDAY = /^(\d{4}|-)-(\d{2})-(\d{2})$/;

/** Parties d'un anniversaire de Rust (`1985-04-12`, `--04-12`, vide). */
export function birthdayParts(value: string): BirthdayParts {
  const found = BIRTHDAY.exec(value);
  if (found === null) return { day: "", month: "", year: "" };
  const [, year = "-", month = "", day = ""] = found;
  return {
    day: String(Number(day)),
    month: String(Number(month)),
    year: year === "-" ? "" : year,
  };
}

/** Anniversaire envoyé à Rust ; vide si rien n'est saisi. */
export function composeBirthday(parts: BirthdayParts): string {
  const day = parts.day.trim();
  const month = parts.month.trim();
  const year = parts.year.trim();
  if (day === "" && month === "" && year === "") return "";
  const two = (text: string) => text.padStart(2, "0");
  // L'année part telle que saisie : « 85 » est refusée par Rust plutôt que
  // lue comme l'an 85.
  return `${year === "" ? "-" : year}-${two(month)}-${two(day)}`;
}

/** Anniversaire en toutes lettres dans la langue `locale` (forme
 * inattendue : telle quelle). */
export function birthdayText(value: string, locale: string): string {
  const found = BIRTHDAY.exec(value);
  if (found === null) return value;
  const [, year = "-", month = "", day = ""] = found;
  const withYear = year !== "-";
  // Année bissextile de repli pour un 29 février sans année.
  const date = new Date(Date.UTC(withYear ? Number(year) : 2000, Number(month) - 1, Number(day)));
  date.setUTCFullYear(withYear ? Number(year) : 2000);
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "long",
    ...(withYear ? { year: "numeric" } : {}),
    timeZone: "UTC",
  }).format(date);
}

/** Noms des douze mois dans la langue `locale`, janvier d'abord. */
export function monthNames(locale: string): string[] {
  const format = new Intl.DateTimeFormat(locale, { month: "long", timeZone: "UTC" });
  return Array.from({ length: 12 }, (_, i) => format.format(new Date(Date.UTC(2000, i, 1))));
}

/** Lignes d'une adresse postale à afficher : rue (sur ses lignes), code
 * postal et ville, région, pays. */
export function postalLines(address: ContactPostalAddressDto): string[] {
  const city = [address.postal_code, address.locality].filter((p) => p !== "").join(" ");
  return [...address.street.split("\n"), city, address.region, address.country].filter(
    (line) => line.trim() !== "",
  );
}
