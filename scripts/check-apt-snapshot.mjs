// Âge de l'instantané apt de l'image e2e (`snapshot=` de app/e2e/Dockerfile),
// lancé par scripts/check.sh. Un instantané trop ancien n'est qu'un
// avertissement : la construction reste reproductible, mais les correctifs
// Ubuntu n'entrent dans l'image qu'avec une montée de la date (procédure :
// docs/technique/tests.md). Une date absente, multiple ou invalide est une
// erreur.
import { readFileSync, realpathSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Âge au-delà duquel une montée de l'instantané est rappelée. */
export const MAX_AGE_DAYS = 35;

// Affectation shell (`snapshot=…` en début de ligne ou après un blanc) ; la
// mention `snapshot=` d'un commentaire, entre accents graves, n'en est pas une.
const SNAPSHOT = /(?<=^|\s)snapshot=(\S+)/gm;
const FORMAT = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/;
const DAY_MS = 24 * 60 * 60 * 1000;

function fail(message) {
  throw new Error(`instantané apt de l'e2e : ${message}`);
}

/** Date de l'instantané, âge en jours entiers et besoin de montée. */
export function snapshotStatus(dockerfile, now) {
  const found = [...dockerfile.matchAll(SNAPSHOT)].map((m) => m[1]);
  if (found.length !== 1) fail(`${found.length} date(s) trouvée(s), une attendue`);
  const snapshot = found[0];
  const parts = FORMAT.exec(snapshot);
  if (!parts) fail(`« ${snapshot} » n'est pas au format AAAAMMJJTHHMMSSZ`);
  const [y, mo, d, h, mi, s] = parts.slice(1).map(Number);
  const date = new Date(Date.UTC(y, mo - 1, d, h, mi, s));
  const valid =
    date.getUTCFullYear() === y &&
    date.getUTCMonth() === mo - 1 &&
    date.getUTCDate() === d &&
    date.getUTCHours() === h &&
    date.getUTCMinutes() === mi &&
    date.getUTCSeconds() === s;
  if (!valid) fail(`« ${snapshot} » n'est pas une date valide`);
  if (date.getTime() > now.getTime()) fail(`« ${snapshot} » est dans le futur`);
  const ageDays = Math.floor((now.getTime() - date.getTime()) / DAY_MS);
  return { snapshot, ageDays, stale: ageDays > MAX_AGE_DAYS };
}

// Lancé directement (et non importé par ses tests), y compris par un
// chemin qui traverse un lien symbolique.
const self = realpathSync(fileURLToPath(import.meta.url));
if (process.argv[1] && realpathSync(path.resolve(process.argv[1])) === self) {
  const file = new URL("../app/e2e/Dockerfile", import.meta.url);
  try {
    const status = snapshotStatus(readFileSync(file, "utf8"), new Date());
    if (status.stale) {
      console.warn(
        `AVERTISSEMENT : instantané apt de l'e2e (${status.snapshot}) vieux de ` +
          `${status.ageDays} jours ; monter la date (docs/technique/tests.md).`,
      );
    } else {
      console.log(`Instantané apt de l'e2e : ${status.snapshot} (${status.ageDays} j).`);
    }
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
