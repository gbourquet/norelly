// Photo d'un contact (ADR-012) : servie en lecture seule par le protocole
// `contact-photo://`, qui ne rend que la photo réencodée à l'import ; à
// défaut, l'initiale du nom.
import { isWindows } from "./agent";

/** Adresse de la photo du contact `id` selon la plateforme. */
export function contactPhotoUrl(id: number, windows = isWindows()): string {
  const base = windows ? "http://contact-photo.localhost/" : "contact-photo://localhost/";
  return `${base}${id}`;
}

/** Initiale affichée à défaut de photo. */
export function contactInitial(name: string): string {
  return name.trim().charAt(0).toUpperCase();
}
