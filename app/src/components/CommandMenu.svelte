<script lang="ts" generics="C">
  // Menu déroulant de commandes du registre (« Plus… ») : icône, libellé
  // et raccourci à droite de chaque entrée.
  import { DropdownMenu } from "bits-ui";
  import type { Snippet } from "svelte";
  import type { CommandRegistry } from "../lib/commands/registry";
  import type { VisibleAction } from "../lib/visible-actions";
  import CommandMenuItems from "./CommandMenuItems.svelte";

  interface Props {
    actions: VisibleAction[];
    registry: CommandRegistry<C>;
    context: C;
    label: string;
    /** Bouton d'ouverture ; reçoit les attributs du déclencheur. */
    trigger: Snippet<[{ props: Record<string, unknown> }]>;
    /** Exécution d'une entrée ; par défaut, la commande dans `context`. */
    run?: (id: string) => void;
    /** Alignement du menu sur son bouton. */
    align?: "start" | "end";
  }

  let { actions, registry, context, label, trigger, run, align = "end" }: Props = $props();
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      {@render trigger({ props })}
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal>
    <DropdownMenu.Content class="menu" aria-label={label} {align} sideOffset={4}>
      <CommandMenuItems kind="dropdown" {actions} {registry} {context} {run} />
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>
