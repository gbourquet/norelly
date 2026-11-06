// Suivi des modifications du registre (non réactif) pour un composant :
// `version` change à chaque enregistrement ou retrait, et une fois à
// l'abonnement, pour rattraper les commandes enregistrées entre le premier
// rendu et l'abonnement.

import { untrack } from "svelte";
import type { CommandRegistry } from "./registry";

export class RegistryWatch<C> {
  version = $state(0);

  /** À créer pendant l'initialisation d'un composant ; `registry` est lu
   * dans l'effet (propriété du composant). */
  constructor(registry: () => CommandRegistry<C>) {
    $effect(() => {
      const off = registry().subscribe(() => (this.version += 1));
      // Hors suivi : l'effet ne dépend pas de `version`.
      untrack(() => (this.version += 1));
      return off;
    });
  }
}
