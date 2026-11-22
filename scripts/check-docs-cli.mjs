// Point d'entrée du vérificateur de documentation (scripts/check-docs.mjs),
// lancé par scripts/check-docs.sh. Sans condition sur le chemin lancé : la
// vérification ne peut pas être sautée en silence.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { checkRepository } from "./check-docs.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const problems = checkRepository(root);
for (const p of problems) console.error(`✗ ${p}`);
if (problems.length > 0) {
  console.error(`\n${problems.length} problème(s) dans la documentation.`);
  process.exit(1);
}
console.log("Documentation : liens, pages et captures en ordre.");
