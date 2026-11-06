<script lang="ts" generics="C">
  // Boutons d'actions (barre d'outils, survol d'une ligne) : chacun exécute
  // une commande du registre ; libellé et raccourci viennent du registre.
  import type { CommandRegistry } from "../lib/commands/registry";
  import { RegistryWatch } from "../lib/commands/watch.svelte";
  import { describeCommand } from "../lib/commands/describe";
  import type { VisibleAction } from "../lib/visible-actions";

  interface Props {
    actions: VisibleAction[];
    registry: CommandRegistry<C>;
    context: C;
    label: string;
    compact?: boolean;
    /** Appelé avant l'exécution (par exemple pour sélectionner la ligne). */
    onBefore?: () => void;
  }

  let { actions, registry, context, label, compact = false, onBefore }: Props = $props();

  // Le registre n'est pas réactif : relecture à chaque modification.
  const watch = new RegistryWatch(() => registry);

  /** Libellé, infobulle « Libellé — Touche » et disponibilité. */
  function describe(action: VisibleAction): { label: string; title: string; enabled: boolean } {
    void watch.version;
    const info = describeCommand(registry, action.command, context, action.label?.());
    if (info === null) return { label: action.command, title: action.command, enabled: false };
    return { label: info.label, title: info.tooltip, enabled: info.enabled };
  }

  function run(event: MouseEvent, id: string): void {
    // Dans une ligne de liste : ne pas ouvrir la conversation.
    event.stopPropagation();
    onBefore?.();
    void registry.execute(id, context);
  }
</script>

<div class="actions" class:compact role="toolbar" aria-label={label}>
  {#each actions as action (action.command)}
    {@const info = describe(action)}
    {@const Icon = action.icon}
    <button
      class="btn btn-ghost icon"
      type="button"
      title={info.title}
      aria-label={info.label}
      disabled={!compact && !info.enabled}
      tabindex={compact ? -1 : 0}
      onclick={(e) => run(e, action.command)}
    >
      <Icon size={compact ? 14 : 16} aria-hidden="true" />
    </button>
  {/each}
</div>

<style>
  .actions {
    display: flex;
    gap: var(--space-1);
  }

  .icon {
    padding: var(--space-1);
    min-height: 0;
  }
</style>
