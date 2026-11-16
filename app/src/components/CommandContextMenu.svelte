<script lang="ts" generics="C">
  // Menu contextuel (clic droit) de commandes du registre, sur un élément :
  // mêmes entrées que son menu « ⋯ » (CommandMenu). Sans entrée, l'élément
  // est affiché seul, sans menu.
  import { ContextMenu } from "bits-ui";
  import type { Snippet } from "svelte";
  import type { CommandRegistry } from "../lib/commands/registry";
  import type { VisibleAction } from "../lib/visible-actions";
  import CommandMenuItems from "./CommandMenuItems.svelte";

  interface Props {
    actions: VisibleAction[];
    registry: CommandRegistry<C>;
    context: C;
    label: string;
    /** Élément qui ouvre le menu ; reçoit les attributs du déclencheur. */
    children: Snippet<[{ props: Record<string, unknown> }]>;
    /** Exécution d'une entrée ; par défaut, la commande dans `context`. */
    run?: (id: string) => void;
  }

  let { actions, registry, context, label, children, run }: Props = $props();

  /** Attributs du déclencheur, sans ceux d'un bouton : l'élément (une
   * ligne de liste) ne devient ni focalisable ni « désactivé ». */
  function triggerProps(props: Record<string, unknown>): Record<string, unknown> {
    return Object.fromEntries(
      Object.entries(props).filter(([key]) => key !== "tabindex" && key !== "disabled"),
    );
  }
</script>

{#if actions.length === 0}
  {@render children({ props: {} })}
{:else}
  <ContextMenu.Root>
    <ContextMenu.Trigger>
      {#snippet child({ props })}
        {@render children({ props: triggerProps(props) })}
      {/snippet}
    </ContextMenu.Trigger>
    <ContextMenu.Portal>
      <ContextMenu.Content class="menu" aria-label={label}>
        <CommandMenuItems kind="context" {actions} {registry} {context} {run} />
      </ContextMenu.Content>
    </ContextMenu.Portal>
  </ContextMenu.Root>
{/if}
