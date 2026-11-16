<script lang="ts" generics="C">
  // Entrées d'un menu de commandes du registre : icône, libellé et
  // raccourci à droite, séparateur entre deux groupes. Partagées par le
  // menu déroulant (« ⋯ », « Plus… ») et le menu du clic droit.
  import { ContextMenu, DropdownMenu } from "bits-ui";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { RegistryWatch } from "../lib/commands/watch.svelte";
  import { describeCommand } from "../lib/commands/describe";
  import type { VisibleAction } from "../lib/visible-actions";

  interface Props {
    /** Menu déroulant ou menu contextuel (bits-ui). */
    kind: "dropdown" | "context";
    actions: VisibleAction[];
    registry: CommandRegistry<C>;
    context: C;
    /** Exécution d'une entrée ; par défaut, la commande dans `context`. */
    run?: (id: string) => void;
  }

  let { kind, actions, registry, context, run }: Props = $props();

  const Item = $derived(kind === "context" ? ContextMenu.Item : DropdownMenu.Item);
  const Separator = $derived(kind === "context" ? ContextMenu.Separator : DropdownMenu.Separator);

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

{#each actions as action, i (action.command)}
  {@const info = describe(action)}
  {@const Icon = action.icon}
  {#if i > 0 && (actions[i - 1]?.section ?? 0) !== (action.section ?? 0)}
    <Separator class="menu-separator" />
  {/if}
  {#if info !== null}
    <Item class="menu-item" disabled={!info.enabled} onSelect={() => select(action.command)}>
      <Icon size={16} aria-hidden="true" />
      <span class="menu-label">{info.label}</span>
      {#if info.keys}<kbd>{info.keys}</kbd>{/if}
    </Item>
  {/if}
{/each}
