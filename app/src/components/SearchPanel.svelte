<script lang="ts">
  // Recherche à la place de la liste des conversations : barre de saisie
  // et messages trouvés. Le surlignage arrive de Rust en morceaux de
  // texte : affichés comme du texte, jamais interprétés.
  import { m } from "../lib/paraglide/messages.js";
  import SearchIcon from "@lucide/svelte/icons/search";
  import XIcon from "@lucide/svelte/icons/x";
  import PaperclipIcon from "@lucide/svelte/icons/paperclip";
  import FlagIcon from "@lucide/svelte/icons/flag";
  import type { SegmentDto } from "../lib/ipc/api";
  import type { SearchState } from "../lib/search.svelte";
  import { personName, shortDate } from "../lib/format";

  interface Props {
    search: SearchState;
    /** Ouvre le résultat sélectionné. */
    onOpen: () => void;
  }

  let { search, onOpen }: Props = $props();

  let input: HTMLInputElement | null = $state(null);
  let results: HTMLElement | null = $state(null);

  // Focus demandé (« / », bouton de la liste).
  $effect(() => {
    if (search.focusRequest > 0) input?.focus();
  });

  // Résultat sélectionné gardé visible.
  $effect(() => {
    const hit = search.selected;
    if (hit === null || results === null) return;
    results.querySelector(`#hit-${hit.message}`)?.scrollIntoView?.({ block: "nearest" });
  });

  function onKey(e: KeyboardEvent): void {
    if (e.isComposing) return;
    if (e.key === "Enter") {
      e.preventDefault();
      void search.submit().then(() => results?.focus());
    } else if (e.key === "Escape") {
      e.preventDefault();
      search.close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      results?.focus();
    }
  }

  function onScroll(e: Event): void {
    const el = e.currentTarget as HTMLElement;
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - 200) void search.loadMore();
  }

  function choose(index: number): void {
    search.select(index);
    onOpen();
  }
</script>

{#snippet segments(list: SegmentDto[])}
  {#each list as s, i (i)}{#if s.hit}<mark>{s.text}</mark>{:else}{s.text}{/if}{/each}
{/snippet}

<div class="search">
  <header>
    <SearchIcon size={16} aria-hidden="true" />
    <input
      bind:this={input}
      type="search"
      aria-label={m.search_label()}
      placeholder={m.search_placeholder()}
      value={search.query}
      oninput={(e) => search.setQuery(e.currentTarget.value)}
      onkeydown={onKey}
      autocomplete="off"
      spellcheck="false"
    />
    <button
      class="btn-icon"
      type="button"
      aria-label={m.search_close()}
      title={m.search_close()}
      onclick={() => search.close()}
    >
      <XIcon size={16} aria-hidden="true" />
    </button>
  </header>
  {#if search.error}
    <p class="error" role="alert">{search.error}</p>
  {:else if search.searched && search.hits.length === 0}
    <p class="empty" role="status">{m.search_none()}</p>
  {/if}
  <div
    class="scroller"
    bind:this={results}
    role="listbox"
    aria-label={m.search_results()}
    tabindex="0"
    aria-activedescendant={search.selected ? `hit-${search.selected.message}` : undefined}
    onscroll={onScroll}
  >
    {#each search.hits as h, index (h.message)}
      <div
        id="hit-{h.message}"
        class="row"
        class:selected={index === search.selectedIndex}
        class:unread={h.unread}
        role="option"
        aria-selected={index === search.selectedIndex}
        tabindex="-1"
        onclick={() => choose(index)}
        onkeydown={(e) => {
          if (e.key === " ") {
            e.preventDefault();
            choose(index);
          }
        }}
      >
        <div class="line">
          <span class="from">{h.from ? personName(h.from) : m.common_unknown_sender()}</span>
          {#if h.has_attachments}<PaperclipIcon size={14} aria-label={m.list_attachment()} />{/if}
          {#if h.flagged}<FlagIcon size={14} aria-label={m.list_flagged()} />{/if}
          <span class="date">{shortDate(h.date)}</span>
        </div>
        <div class="line subject">
          {#if h.subject.length > 0}{@render segments(h.subject)}{:else}{m.common_no_subject()}{/if}
        </div>
        {#if h.snippet.length > 0}
          <div class="snippet">{@render segments(h.snippet)}</div>
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .search {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-bottom: var(--border-width) solid var(--color-border);
    color: var(--color-text-muted);
  }

  input {
    flex: 1 1 auto;
    min-width: 0;
    padding: var(--space-1) var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
  }

  input:focus-visible {
    outline: 2px solid var(--color-focus);
    outline-offset: 1px;
  }

  .scroller {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    outline: none;
  }

  .scroller:focus-visible {
    box-shadow: inset 0 0 0 2px var(--color-focus);
  }

  .row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2) var(--space-4);
    border-bottom: var(--border-width) solid var(--color-border);
    cursor: pointer;
  }

  .row:hover {
    background: var(--color-hover);
  }

  .row.selected {
    background: var(--color-selected);
  }

  .line {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .from,
  .subject,
  .snippet {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .from {
    flex: 1 1 auto;
  }

  .subject {
    display: block;
    font-size: var(--font-size-sm);
  }

  .snippet {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .unread .from,
  .unread .subject {
    font-weight: var(--font-weight-bold);
  }

  .date {
    flex: none;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  mark {
    border-radius: 2px;
    background: var(--color-highlight, color-mix(in srgb, var(--color-accent) 25%, transparent));
    color: inherit;
  }

  .empty,
  .error {
    padding: var(--space-4);
    color: var(--color-text-muted);
  }

  .error {
    color: var(--color-danger);
  }
</style>
