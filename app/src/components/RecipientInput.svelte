<script lang="ts">
  // Champ de destinataires : puces retirables, saisie validée par Entrée,
  // virgule ou tabulation, suggestions (contacts du carnet, signalés, puis
  // adresses déjà utilisées ; liste accessible au clavier). La saisie est
  // lue et validée en Rust, qui écarte aussi tout nom imitant une adresse.
  import BookUserIcon from "@lucide/svelte/icons/book-user";
  import XIcon from "@lucide/svelte/icons/x";
  import type { RecipientDto, RecipientSuggestion } from "../lib/ipc/api";
  import { m } from "../lib/paraglide/messages.js";

  interface Props {
    label: string;
    recipients: RecipientDto[];
    suggestions: RecipientSuggestion[];
    /** Résout `false` si la saisie est refusée. */
    onAdd: (raw: string) => Promise<boolean>;
    onRemove: (index: number) => void;
    onQuery: (text: string) => void;
  }

  let { label, recipients, suggestions, onAdd, onRemove, onQuery }: Props = $props();

  const id = $props.id();
  let text = $state("");
  let focused = $state(false);
  let highlighted = $state(-1);
  const shown = $derived(focused && text.trim() !== "" ? suggestions : []);

  async function commit(raw: string): Promise<void> {
    if (raw.trim() === "") return;
    const typed = text;
    if (await onAdd(raw)) {
      // Saisie poursuivie pendant la lecture : gardée.
      if (text !== typed) return;
      text = "";
      highlighted = -1;
      onQuery("");
    }
  }

  /** « Nom <adresse> », ou l'adresse seule si Rust n'a pas gardé de nom. */
  const format = (r: RecipientDto): string => (r.name ? `${r.name} <${r.address}>` : r.address);

  function onKeyDown(e: KeyboardEvent): void {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === "ArrowDown" && shown.length > 0) {
      e.preventDefault();
      highlighted = (highlighted + 1) % shown.length;
    } else if (e.key === "ArrowUp" && shown.length > 0) {
      e.preventDefault();
      highlighted = (highlighted - 1 + shown.length) % shown.length;
    } else if (e.key === "Enter" || e.key === "," || (e.key === "Tab" && text.trim() !== "")) {
      const choice = shown[highlighted];
      if (choice !== undefined || text.trim() !== "") e.preventDefault();
      void commit(choice !== undefined ? format(choice) : text);
    } else if (e.key === "Backspace" && text === "" && recipients.length > 0) {
      onRemove(recipients.length - 1);
    }
  }
</script>

<div class="field">
  <label for={id}>{label}</label>
  <div class="chips">
    {#each recipients as r, i (r.address)}
      <span class="chip" title={r.address}>
        {format(r)}
        <button
          type="button"
          aria-label={m.compose_remove_recipient({ address: r.address })}
          onclick={() => onRemove(i)}><XIcon size={12} /></button
        >
      </span>
    {/each}
    <input
      {id}
      type="text"
      role="combobox"
      autocomplete="off"
      aria-label={label}
      aria-expanded={shown.length > 0}
      aria-controls="{id}-list"
      aria-autocomplete="list"
      bind:value={text}
      oninput={() => onQuery(text)}
      onkeydown={onKeyDown}
      onfocus={() => (focused = true)}
      onblur={() => {
        focused = false;
        void commit(text);
      }}
    />
  </div>
  {#if shown.length > 0}
    <ul id="{id}-list" role="listbox" aria-label={m.compose_suggestions()}>
      {#each shown as s, i (s.address)}
        <li role="option" aria-selected={i === highlighted}>
          <button
            type="button"
            tabindex="-1"
            title={s.address}
            onmousedown={(e) => e.preventDefault()}
            onclick={() => void commit(format(s))}
          >
            <span class="who">{format(s)}</span>
            {#if s.contact != null}
              <span class="in-contacts">
                <BookUserIcon size={12} aria-hidden="true" />
                {m.compose_suggestion_contact()}
              </span>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .field {
    position: relative;
    display: grid;
    grid-template-columns: 4rem 1fr;
    align-items: center;
    gap: var(--space-2);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    padding: var(--space-1);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--color-border);
  }

  .chip button {
    display: inline-flex;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  input:focus-visible {
    box-shadow: inset 0 -2px 0 var(--color-focus);
  }

  input {
    flex: 1;
    min-width: 10rem;
    border: none;
    background: transparent;
    color: inherit;
    outline: none;
  }

  ul {
    position: absolute;
    top: 100%;
    left: 4rem;
    right: 0;
    z-index: 2;
    margin: 0;
    padding: var(--space-1);
    list-style: none;
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-surface-raised);
  }

  li button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-1) var(--space-2);
    border: none;
    background: transparent;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }

  .who {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .in-contacts {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    gap: var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  li button:hover {
    background: var(--color-border);
  }

  li[aria-selected="true"] button {
    background: var(--color-border);
  }
</style>
