<script lang="ts" generics="C">
  // Barre d'actions libellées (icône et texte) : chaque bouton exécute une
  // commande du registre. Les actions secondaires, et les principales qui
  // ne tiennent pas dans la largeur, vont dans le menu « Plus… ».
  import EllipsisIcon from "@lucide/svelte/icons/ellipsis";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { RegistryWatch } from "../lib/commands/watch.svelte";
  import { describeCommand } from "../lib/commands/describe";
  import { fitByPriority, type VisibleAction } from "../lib/visible-actions";
  import { m } from "../lib/paraglide/messages.js";
  import CommandMenu from "./CommandMenu.svelte";

  interface Props {
    primary: VisibleAction[];
    more: VisibleAction[];
    registry: CommandRegistry<C>;
    context: C;
    label: string;
  }

  let { primary, more, registry, context, label }: Props = $props();

  // Le registre n'est pas réactif : relecture à chaque modification.
  const watch = new RegistryWatch(() => registry);

  function describe(action: VisibleAction) {
    void watch.version;
    return describeCommand(registry, action.command, context, action.label?.());
  }

  // Largeurs mesurées sur une copie invisible de la barre.
  let available = $state(0);
  let widths = $state<number[]>([]);
  let moreWidth = $state(0);
  let gap = $state(0);
  let bar: HTMLElement | null = $state(null);

  $effect(() => {
    if (bar !== null) gap = parseFloat(getComputedStyle(bar).columnGap) || 0;
  });

  const visible = $derived(
    fitByPriority(
      primary.map((a, i) => ({ width: widths[i] ?? 0, priority: a.priority ?? i + 1 })),
      available,
      moreWidth,
      gap,
    ),
  );
  const overflow = $derived([...primary.filter((_, i) => !visible[i]), ...more]);

  function run(id: string): void {
    void registry.execute(id, context);
  }
</script>

<div class="bar" role="toolbar" aria-label={label} bind:this={bar} bind:clientWidth={available}>
  {#each primary as action, i (action.command)}
    {@const info = describe(action)}
    {@const Icon = action.icon}
    {#if info !== null && visible[i]}
      <button
        class="btn"
        type="button"
        title={info.tooltip}
        disabled={!info.enabled}
        onclick={() => run(action.command)}
      >
        <Icon size={16} aria-hidden="true" />
        <span>{info.label}</span>
      </button>
    {/if}
  {/each}
  {#if overflow.length > 0}
    <CommandMenu actions={overflow} {registry} {context} label={m.action_more()}>
      {#snippet trigger({ props })}
        <button {...props} class="btn" type="button" title={m.action_more()}>
          <EllipsisIcon size={16} aria-hidden="true" />
          <span>{m.action_more()}</span>
        </button>
      {/snippet}
    </CommandMenu>
  {/if}

  <!-- Copie invisible : largeur naturelle de chaque bouton. -->
  <div class="measure" aria-hidden="true" inert>
    {#each primary as action, i (action.command)}
      {@const info = describe(action)}
      {@const Icon = action.icon}
      <span class="btn" bind:offsetWidth={widths[i]}>
        <Icon size={16} aria-hidden="true" />
        <span>{info?.label}</span>
      </span>
    {/each}
    <span class="btn" bind:offsetWidth={moreWidth}>
      <EllipsisIcon size={16} aria-hidden="true" />
      <span>{m.action_more()}</span>
    </span>
  </div>
</div>

<style>
  .bar {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .bar .btn {
    flex: none;
    white-space: nowrap;
  }

  .measure {
    position: absolute;
    top: 0;
    left: 0;
    display: flex;
    gap: var(--space-2);
    height: 0;
    overflow: hidden;
    visibility: hidden;
    pointer-events: none;
  }
</style>
