<script lang="ts" generics="C">
  // Barre d'actions libellées (icône et texte) : chaque bouton exécute une
  // commande du registre. Les actions secondaires, et les principales qui
  // ne tiennent pas dans la largeur (sur `rows` rangées, une par défaut),
  // vont dans le menu « Plus… ».
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
    rows?: number;
  }

  let { primary, more, registry, context, label, rows = 1 }: Props = $props();

  // Le registre n'est pas réactif : relecture à chaque modification.
  const watch = new RegistryWatch(() => registry);

  function describe(action: VisibleAction) {
    void watch.version;
    return describeCommand(registry, action.command, context, action.label?.());
  }

  // Largeurs mesurées sur une copie invisible de la barre, sans arrondi :
  // boutons arrondis au pixel supérieur, place disponible au pixel
  // inférieur, pour qu'une rangée jugée pleine le soit aussi à l'écran.
  type Box = readonly ResizeObserverSize[] | undefined;
  let barRect = $state<DOMRectReadOnly>();
  let boxes = $state<Box[]>([]);
  let moreBox = $state<Box>();
  let gap = $state(0);
  let bar: HTMLElement | null = $state(null);
  const outer = (box: Box): number => Math.ceil(box?.[0]?.inlineSize ?? 0);

  $effect(() => {
    if (bar !== null) gap = parseFloat(getComputedStyle(bar).columnGap) || 0;
  });

  const visible = $derived(
    fitByPriority(
      primary.map((a, i) => ({
        width: outer(boxes[i]),
        priority: a.priority ?? i + 1,
        absent: describe(a) === null,
      })),
      Math.floor(barRect?.width ?? 0),
      outer(moreBox),
      gap,
      more.length > 0,
      rows,
    ),
  );
  const overflow = $derived([
    ...primary.filter((a, i) => !visible[i] && describe(a) !== null),
    ...more,
  ]);

  function run(id: string): void {
    void registry.execute(id, context);
  }
</script>

<div
  class="bar"
  class:wrap={rows > 1}
  role="toolbar"
  aria-label={label}
  bind:this={bar}
  bind:contentRect={barRect}
>
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
      <span class="btn" bind:borderBoxSize={boxes[i]}>
        <Icon size={16} aria-hidden="true" />
        <span>{info?.label}</span>
      </span>
    {/each}
    <span class="btn" bind:borderBoxSize={moreBox}>
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

  .bar.wrap {
    flex-wrap: wrap;
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
