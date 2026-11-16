<script lang="ts" generics="C">
  // Menu contextuel (clic droit) de commandes du registre, sur un élément :
  // mêmes entrées que son menu « ⋯ » (CommandMenu). Sans entrée, l'élément
  // est affiché seul, sans menu.
  import { ContextMenu } from "bits-ui";
  import type { Snippet } from "svelte";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { RegistryWatch } from "../lib/commands/watch.svelte";
  import { describeCommand } from "../lib/commands/describe";
  import type { VisibleAction } from "../lib/visible-actions";

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

  function select(id: string): void {
    if (run) run(id);
    else void registry.execute(id, context);
  }

  // Le registre n'est pas réactif : relecture à chaque modification.
  const watch = new RegistryWatch(() => registry);

  function describe(action: VisibleAction) {
    void watch.version;
    return describeCommand(registry, action.command, context, action.label?.());
  }
</script>

{#if actions.length === 0}
  {@render children({ props: {} })}
{:else}
  <ContextMenu.Root>
    <ContextMenu.Trigger>
      {#snippet child({ props })}
        {@render children({ props })}
      {/snippet}
    </ContextMenu.Trigger>
    <ContextMenu.Portal>
      <ContextMenu.Content class="menu" aria-label={label}>
        {#each actions as action, i (action.command)}
          {@const info = describe(action)}
          {@const Icon = action.icon}
          {#if i > 0 && (actions[i - 1]?.section ?? 0) !== (action.section ?? 0)}
            <ContextMenu.Separator class="menu-separator" />
          {/if}
          {#if info !== null}
            <ContextMenu.Item
              class="menu-item"
              disabled={!info.enabled}
              onSelect={() => select(action.command)}
            >
              <Icon size={16} aria-hidden="true" />
              <span class="menu-label">{info.label}</span>
              {#if info.keys}<kbd>{info.keys}</kbd>{/if}
            </ContextMenu.Item>
          {/if}
        {/each}
      </ContextMenu.Content>
    </ContextMenu.Portal>
  </ContextMenu.Root>
{/if}
