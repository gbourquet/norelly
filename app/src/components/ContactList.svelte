<script lang="ts" generics="C">
  import { m } from "../lib/paraglide/messages.js";
  // Liste virtualisée du carnet d'adresses : recherche, filtre des
  // favoris, cases à cocher et menu contextuel de chaque ligne.
  import StarIcon from "@lucide/svelte/icons/star";
  import XIcon from "@lucide/svelte/icons/x";
  import SearchIcon from "@lucide/svelte/icons/search";
  import UserPlusIcon from "@lucide/svelte/icons/user-plus";
  import { ContextMenu } from "bits-ui";
  import { untrack } from "svelte";
  import type { ContactsState } from "../lib/contacts.svelte";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { RegistryWatch } from "../lib/commands/watch.svelte";
  import { describeCommand } from "../lib/commands/describe";
  import { contactBulkActions, contactMenuActions } from "../lib/visible-actions";
  import ActionBar from "./ActionBar.svelte";
  import { VirtualList } from "../lib/virtual-list.svelte";

  interface Props {
    contacts: ContactsState;
    registry: CommandRegistry<C>;
    context: C;
    /** Contact visé par le menu contextuel (`null` : menu fermé). */
    onPoint: (id: number | null) => void;
  }

  let { contacts, registry, context, onPoint }: Props = $props();

  // Le registre n'est pas réactif : relecture à chaque modification.
  const watch = new RegistryWatch(() => registry);

  function describe(id: string, label?: string) {
    void watch.version;
    return describeCommand(registry, id, context, label);
  }

  const bulk = contactBulkActions();
  const create = $derived(describe("contacts.new"));

  /** Contact dont le menu contextuel est ouvert. */
  let pointed = $state<number | null>(null);
  const menu = $derived(
    contactMenuActions({
      favorite: contacts.items.find((i) => i.id === pointed)?.favorite ?? false,
    }),
  );

  const ROW_HEIGHT = 52;
  let scroller: HTMLElement | null = $state(null);
  const list = new VirtualList(() => scroller, ROW_HEIGHT);

  $effect(() => list.mount());
  $effect(() => list.setCount(contacts.items.length));
  $effect(() => {
    const last = list.items.at(-1);
    if (last !== undefined && contacts.hasMore && last.index >= contacts.items.length - 20) {
      void contacts.loadMore();
    }
  });

  // Saisie de la recherche : liste relue après une courte pause.
  // Texte en cours de saisie, parti du filtre en place à l'affichage.
  let typed = $state(untrack(() => contacts.text));
  let timer: ReturnType<typeof setTimeout> | null = null;
  function onInput(): void {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      void contacts.search(typed);
    }, 200);
  }

  function point(id: number | null): void {
    pointed = id;
    onPoint(id);
  }

  const selectedIndex = $derived(contacts.items.findIndex((i) => i.id === contacts.selectedId));
</script>

<div class="contacts">
  <header>
    <h2>{m.contacts_title()}</h2>
    <button
      class="btn btn-primary"
      type="button"
      title={create?.tooltip}
      onclick={() => void registry.execute("contacts.new", context)}
    >
      <UserPlusIcon size={16} aria-hidden="true" />
      <span>{create?.label}</span>
    </button>
  </header>
  <div class="filters">
    <label class="search">
      <SearchIcon size={16} aria-hidden="true" />
      <input
        class="input"
        type="search"
        placeholder={m.contacts_search_placeholder()}
        aria-label={m.contacts_search_label()}
        bind:value={typed}
        oninput={onInput}
      />
    </label>
    <div class="segmented" role="group" aria-label={m.contacts_filter_label()}>
      <button
        class="btn"
        type="button"
        aria-pressed={!contacts.favoritesOnly}
        onclick={() => void contacts.showFavorites(false)}>{m.contacts_filter_all()}</button
      >
      <button
        class="btn"
        type="button"
        aria-pressed={contacts.favoritesOnly}
        onclick={() => void contacts.showFavorites(true)}
      >
        <StarIcon size={14} aria-hidden="true" />
        <span>{m.contacts_filter_favorites()}</span>
      </button>
    </div>
  </div>
  {#if contacts.marked.length > 0}
    {@const clear = describe("contacts.clear-selection")}
    <section class="bulk" aria-label={m.contacts_selection()}>
      <div class="bulk-head">
        <span class="marked-count" role="status"
          >{m.contacts_marked({ count: contacts.marked.length })}</span
        >
        <button
          class="btn btn-ghost"
          type="button"
          title={clear?.tooltip}
          onclick={() => void registry.execute("contacts.clear-selection", context)}
        >
          <XIcon size={16} aria-hidden="true" />
          <span>{clear?.label}</span>
        </button>
      </div>
      <ActionBar
        primary={bulk.primary}
        more={bulk.more}
        {registry}
        {context}
        label={m.contacts_bulk_actions()}
      />
    </section>
  {/if}
  {#if contacts.error}
    <p class="error" role="alert">{contacts.error}</p>
  {/if}
  {#if contacts.items.length === 0 && !contacts.loading}
    <p class="empty">
      {contacts.text !== "" || contacts.favoritesOnly
        ? m.contacts_none_found()
        : m.contacts_empty()}
    </p>
  {/if}
  <ContextMenu.Root onOpenChange={(open) => (open ? undefined : point(null))}>
    <ContextMenu.Trigger>
      {#snippet child({ props })}
        <div
          {...props}
          class="scroller"
          bind:this={scroller}
          role="listbox"
          aria-label={m.contacts_list()}
          aria-multiselectable={contacts.marked.length > 0 ? "true" : undefined}
          tabindex="0"
          aria-activedescendant={contacts.selectedId !== null
            ? `contact-${contacts.selectedId}`
            : undefined}
        >
          <div class="spacer" style:height="{list.totalSize}px">
            {#each list.items as row (row.key)}
              {@const c = contacts.items[row.index]}
              {#if c !== undefined}
                <div
                  id="contact-{c.id}"
                  class="row"
                  class:selected={row.index === selectedIndex}
                  class:marked={contacts.marked.includes(c.id)}
                  role="option"
                  aria-selected={contacts.marked.length > 0
                    ? contacts.marked.includes(c.id)
                    : row.index === selectedIndex}
                  tabindex="-1"
                  style:height="{ROW_HEIGHT}px"
                  style:transform="translateY({row.start}px)"
                  onclick={() => void contacts.select(c.id)}
                  oncontextmenu={() => point(c.id)}
                  onkeydown={(e) => {
                    if (e.key === " " || e.key === "Enter") {
                      e.preventDefault();
                      void contacts.select(c.id);
                    }
                  }}
                >
                  <input
                    class="check"
                    type="checkbox"
                    tabindex="-1"
                    checked={contacts.marked.includes(c.id)}
                    aria-label={m.contacts_select_row({ name: c.name })}
                    title={m.contacts_mark()}
                    onclick={(e) => {
                      // Cocher n'ouvre pas la fiche.
                      e.stopPropagation();
                      contacts.toggleMark(c.id);
                    }}
                  />
                  <div class="content">
                    <div class="line">
                      <span class="name">{c.name}</span>
                      {#if c.favorite}<span class="star"
                          ><StarIcon size={14} aria-label={m.contacts_is_favorite()} /></span
                        >{/if}
                    </div>
                    <div class="line">
                      <span class="detail"
                        >{c.organization ? `${c.address} · ${c.organization}` : c.address}</span
                      >
                    </div>
                  </div>
                </div>
              {/if}
            {/each}
          </div>
        </div>
      {/snippet}
    </ContextMenu.Trigger>
    <ContextMenu.Portal>
      <ContextMenu.Content class="menu" aria-label={m.contacts_context_menu()}>
        {#each menu as action, i (action.command)}
          {@const e = describe(action.command, action.label?.())}
          {@const Icon = action.icon}
          {#if i > 0 && (menu[i - 1]?.section ?? 0) !== (action.section ?? 0)}
            <ContextMenu.Separator class="menu-separator" />
          {/if}
          {#if e !== null}
            <ContextMenu.Item
              class="menu-item"
              disabled={!e.enabled}
              onSelect={() => void registry.execute(action.command, context)}
            >
              <Icon size={16} aria-hidden="true" />
              <span class="menu-label">{e.label}</span>
              {#if e.keys}<kbd>{e.keys}</kbd>{/if}
            </ContextMenu.Item>
          {/if}
        {/each}
      </ContextMenu.Content>
    </ContextMenu.Portal>
  </ContextMenu.Root>
</div>

<style>
  .contacts {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-bottom: var(--border-width) solid var(--color-border);
  }

  h2 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-medium);
  }

  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-bottom: var(--border-width) solid var(--color-border);
  }

  .search {
    display: flex;
    flex: 1 1 160px;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    color: var(--color-text-muted);
  }

  .search .input {
    flex: 1;
    min-width: 0;
  }

  .segmented {
    display: flex;
    gap: var(--space-1);
  }

  .segmented .btn[aria-pressed="true"] {
    border-color: var(--color-accent);
    background: var(--color-selected);
  }

  .bulk {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-bottom: var(--border-width) solid var(--color-border);
    background: var(--color-surface);
  }

  .bulk-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .marked-count {
    font-weight: var(--font-weight-medium);
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

  .spacer {
    position: relative;
    width: 100%;
  }

  .row {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-4) 0 var(--space-3);
    border-bottom: var(--border-width) solid var(--color-border);
    cursor: pointer;
    box-sizing: border-box;
  }

  .row:hover {
    background: var(--color-hover);
  }

  .row.selected {
    background: var(--color-selected);
  }

  .row.marked {
    box-shadow: inset 3px 0 0 var(--color-accent);
  }

  .check {
    flex: none;
    margin: 0;
    accent-color: var(--color-accent);
    cursor: pointer;
  }

  .content {
    display: flex;
    flex: 1;
    flex-direction: column;
    justify-content: center;
    gap: var(--space-1);
    min-width: 0;
  }

  .line {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .name,
  .detail {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name {
    font-weight: var(--font-weight-medium);
  }

  .detail {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .star {
    display: inline-flex;
    flex: none;
    color: var(--color-warning);
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
