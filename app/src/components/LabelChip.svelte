<script lang="ts">
  // Pastille d'une étiquette : nom (texte échappé) sur sa couleur. Les
  // couleurs viennent de Rust (`#rrggbb` validé, texte blanc ou noir choisi
  // pour le contraste) et sont contrôlées de nouveau avant tout style ;
  // sans couleur, la pastille suit les jetons du thème. Un clic cherche les
  // messages de l'étiquette.
  import { m } from "../lib/paraglide/messages.js";
  import type { LabelDto } from "../lib/ipc/api";
  import { chipColors, labelText } from "../lib/labels.svelte";

  interface Props {
    label: LabelDto;
    /** Lance la recherche de l'étiquette ; absent : pastille inerte. */
    onSearch?: (label: LabelDto) => void;
  }

  let { label, onSearch }: Props = $props();

  const text = $derived(labelText(label));
  const colors = $derived(chipColors(label));
</script>

{#if onSearch}
  <button
    class="chip"
    class:colored={colors !== null}
    type="button"
    style:background-color={colors?.background}
    style:color={colors?.text}
    title={m.label_search({ label: text })}
    aria-label={m.label_search({ label: text })}
    onpointerdown={(e) => e.stopPropagation()}
    onclick={(e) => {
      // Le clic ne sélectionne ni n'ouvre la ligne.
      e.stopPropagation();
      onSearch(label);
    }}>{text}</button
  >
{:else}
  <span
    class="chip"
    class:colored={colors !== null}
    style:background-color={colors?.background}
    style:color={colors?.text}>{text}</span
  >
{/if}

<style>
  .chip {
    display: inline-block;
    flex: none;
    max-width: 12em;
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-lg);
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    line-height: 1.6;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Pastille colorée : sa propre couleur de fond ; la bordure la détache
     d'un fond de même teinte, en thème clair comme sombre. */
  .chip.colored {
    border-color: var(--color-border-strong);
  }

  button.chip {
    cursor: pointer;
  }

  button.chip:hover {
    filter: brightness(0.92);
  }

  button.chip:focus-visible {
    outline: 2px solid var(--color-focus);
    outline-offset: 1px;
  }
</style>
