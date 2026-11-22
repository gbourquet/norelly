// Point d'entrée d'un script Node : vrai si le module d'URL `metaUrl` est
// le script lancé (`argv1`, par défaut process.argv[1]). Les deux chemins
// sont résolus (liens symboliques, casse du lecteur sous Windows) : un
// chemin lancé par un lien ne doit pas faire sauter la vérification.
import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

export function isEntryPoint(metaUrl, argv1 = process.argv[1]) {
  if (!argv1) return false;
  try {
    return realpathSync(argv1) === realpathSync(fileURLToPath(metaUrl));
  } catch {
    return false;
  }
}
