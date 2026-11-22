// Point d'entrée du contrôle des versions figées (scripts/check-toolchain.mjs),
// lancé par scripts/check.sh. Sans condition sur le chemin lancé : la
// vérification ne peut pas être sautée en silence.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { checkRepository } from "./check-toolchain.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const problems = checkRepository(root);
for (const p of problems) console.error(`✗ ${p}`);
if (problems.length > 0) {
  console.error(`\n${problems.length} version(s) non figée(s) (procédure : docs/technique/methode.md).`);
  process.exit(1);
}
console.log("Chaîne Rust et outils de la CI : versions figées et cohérentes.");
