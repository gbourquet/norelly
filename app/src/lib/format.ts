// Mise en forme des dates et tailles (affichage seulement).

const time = new Intl.DateTimeFormat("fr-FR", { hour: "2-digit", minute: "2-digit" });
const dayMonth = new Intl.DateTimeFormat("fr-FR", { day: "numeric", month: "short" });
const full = new Intl.DateTimeFormat("fr-FR", {
  day: "2-digit",
  month: "2-digit",
  year: "numeric",
});
const long = new Intl.DateTimeFormat("fr-FR", { dateStyle: "full", timeStyle: "short" });

/** Date courte d'une ligne de liste (secondes Unix). */
export function shortDate(seconds: number, now = new Date()): string {
  const d = new Date(seconds * 1000);
  if (d.toDateString() === now.toDateString()) return time.format(d);
  if (d.getFullYear() === now.getFullYear()) return dayMonth.format(d);
  return full.format(d);
}

/** Date complète (en-tête d'un message). */
export function longDate(seconds: number): string {
  return long.format(new Date(seconds * 1000));
}

/** Taille lisible : « 12 Ko », « 3,4 Mo ». */
export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} o`;
  const units = ["Ko", "Mo", "Go"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = value < 10 ? 1 : 0;
  return `${value.toLocaleString("fr-FR", { maximumFractionDigits: digits })} ${units[unit]}`;
}

/** Nom affichable d'un correspondant. */
export function personName(p: { name: string; address: string } | null | undefined): string {
  if (p === null || p === undefined) return "(expéditeur inconnu)";
  return p.name !== "" ? p.name : p.address;
}
