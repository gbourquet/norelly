<script lang="ts">
  // Champ de destinataires : puces retirables, saisie validée par Entrée,
  // virgule ou tabulation, suggestions (listes de diffusion du carnet,
  // contacts du carnet, signalés, puis adresses déjà utilisées ; liste
  // accessible au clavier). La saisie est lue et validée en Rust, qui
  // écarte aussi tout nom imitant une adresse ; une liste choisie est
  // dépliée par Rust en puces.
  import BookUserIcon from "@lucide/svelte/icons/book-user";
  import UsersIcon from "@lucide/svelte/icons/users";
  import XIcon from "@lucide/svelte/icons/x";
  import type { ContactListItemDto, RecipientDto, RecipientSuggestion } from "../lib/ipc/api";
  import { m } from "../lib/paraglide/messages.js";

  interface Props {
    label: string;
    recipients: RecipientDto[];
    suggestions: RecipientSuggestion[];
    /** Listes de diffusion proposées (avant les personnes). */
    lists?: ContactListItemDto[];
    /** Résout `false` si la saisie est refusée. */
    onAdd: (raw: string) => Promise<boolean>;
    /** Liste choisie ; résout `false` si elle est refusée. */
    onAddList?: (id: number) => Promise<boolean>;
    onRemove: (index: number) => void;
    onQuery: (text: string) => void;
  }

  let {
    label,
    recipients,
    suggestions,
    lists = [],
    onAdd,
    onAddList = async () => false,
    onRemove,
    onQuery,
  }: Props = $props();

  /** Proposition : une liste à déplier, ou une personne. */
  type Option =
    | { kind: "list"; key: string; list: ContactListItemDto }
    | { kind: "person"; key: string; person: RecipientSuggestion };

  const id = $props.id();
  let text = $state("");
  let focused = $state(false);
  let highlighted = $state(-1);
  const shown = $derived<Option[]>(
    focused && text.trim() !== ""
      ? [
          ...lists.map((list) => ({ kind: "list" as const, key: `list-${list.id}`, list })),
          ...suggestions.map((person) => ({
            kind: "person" as const,
            key: `person-${person.address}`,
            person,
          })),
        ]
      : [],
  );

  /** Saisie vidée après un ajout accepté (sauf si elle a changé entre-temps). */
  async function accepted(typed: string, pending: Promise<boolean>): Promise<void> {
    if (!(await pending)) return;
    // Saisie poursuivie pendant la lecture : gardée.
    if (text !== typed) return;
    text = "";
    highlighted = -1;
    onQuery("");
  }

  async function commit(raw: string): Promise<void> {
    if (raw.trim() === "") return;
    await accepted(text, onAdd(raw));
  }

  async function choose(option: Option): Promise<void> {
    if (option.kind === "list") await accepted(text, onAddList(option.list.id));
    else await commit(format(option.person));
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
      void (choice !== undefined ? choose(choice) : commit(text));
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
      {#each shown as option, i (option.key)}
        <li role="option" aria-selected={i === highlighted}>
          {#if option.kind === "list"}
            {@const l = option.list}
            <button
              type="button"
              tabindex="-1"
              title={m.compose_list_option({ name: l.name, count: l.members })}
              onmousedown={(e) => e.preventDefault()}
              onclick={() => void choose(option)}
            >
              <UsersIcon size={14} aria-hidden="true" />
              <span class="who">{l.name}</span>
              <span class="address">{m.lists_members_count({ count: l.members })}</span>
              {#if l.bcc}<span class="tag">{m.lists_bcc_tag()}</span>{/if}
              <span class="in-contacts">{m.compose_suggestion_list()}</span>
            </button>
          {:else}
            {@const s = option.person}
            <button
              type="button"
              tabindex="-1"
              title={s.address}
              onmousedown={(e) => e.preventDefault()}
              onclick={() => void choose(option)}
            >
              <!-- Nom seul tronqué : l'adresse reste toujours entière. -->
              {#if s.name}<span class="who">{s.name}</span>{/if}
              <span class="address">{s.name ? `<${s.address}>` : s.address}</span>
              {#if s.contact != null}
                <span class="in-contacts">
                  <BookUserIcon size={12} aria-hidden="true" />
                  {m.compose_suggestion_contact()}
                </span>
              {/if}
            </button>
          {/if}
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
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .address {
    flex-shrink: 0;
    color: var(--color-text-muted);
  }

  .tag {
    flex-shrink: 0;
    padding: 0 var(--space-1);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .in-contacts {
    display: inline-flex;
    flex-shrink: 0;
    margin-left: auto;
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
