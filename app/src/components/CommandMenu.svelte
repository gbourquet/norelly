<script lang="ts" generics="C">
  // Menu déroulant de commandes du registre (« Plus… ») : icône, libellé
  // et raccourci à droite de chaque entrée.
  import { DropdownMenu } from "bits-ui";
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
    /** Bouton d'ouverture ; reçoit les attributs du déclencheur. */
    trigger: Snippet<[{ props: Record<string, unknown> }]>;
    /** Exécution d'une entrée ; par défaut, la commande dans `context`. */
    run?: (id: string) => void;
    /** Alignement du menu sur son bouton. */
    align?: "start" | "end";
  }

  let { actions, registry, context, label, trigger, run, align = "end" }: Props = $props();

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

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      {@render trigger({ props })}
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal>
    <DropdownMenu.Content class="menu" aria-label={label} {align} sideOffset={4}>
      {#each actions as action, i (action.command)}
        {@const info = describe(action)}
        {@const Icon = action.icon}
        {#if i > 0 && (actions[i - 1]?.section ?? 0) !== (action.section ?? 0)}
          <DropdownMenu.Separator class="menu-separator" />
        {/if}
        {#if info !== null}
          <DropdownMenu.Item
            class="menu-item"
            disabled={!info.enabled}
            onSelect={() => select(action.command)}
          >
            <Icon size={16} aria-hidden="true" />
            <span class="menu-label">{info.label}</span>
            {#if info.keys}<kbd>{info.keys}</kbd>{/if}
          </DropdownMenu.Item>
        {/if}
      {/each}
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>
