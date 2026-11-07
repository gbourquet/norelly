<script lang="ts" generics="C">
  import { m } from "../lib/paraglide/messages.js";
  // Liste virtualisée des conversations : seules les lignes visibles sont
  // dans le DOM, quelle que soit la taille de la boîte.
  import PaperclipIcon from "@lucide/svelte/icons/paperclip";
  import FlagIcon from "@lucide/svelte/icons/flag";
  import XIcon from "@lucide/svelte/icons/x";
  import SearchIcon from "@lucide/svelte/icons/search";
  import { ContextMenu } from "bits-ui";
  import type { MailState } from "../lib/mail.svelte";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { RegistryWatch } from "../lib/commands/watch.svelte";
  import { describeCommand } from "../lib/commands/describe";
  import { bulkActions, menuActions, rowActions, subjectOf } from "../lib/visible-actions";
  import ActionButtons from "./ActionButtons.svelte";
  import ActionBar from "./ActionBar.svelte";
  import { VirtualList } from "../lib/virtual-list.svelte";
  import { personName, shortDate } from "../lib/format";
  import { dragPayload, type DragMove } from "../lib/drag-move.svelte";

  interface Props {
    mail: MailState;
    title: string;
    registry: CommandRegistry<C>;
    context: C;
    /** Ouvre la recherche (bouton de l'en-tête). */
    onSearch?: () => void;
    /** Glisser-déposer des conversations sur un dossier. */
    drag?: DragMove;
  }

  let { mail, title, registry, context, onSearch, drag }: Props = $props();

  /** Entrées du menu contextuel : ouvrir, puis les actions de la barre
   * du mail ouvert, pour la conversation visée. */
  const menu = $derived(menuActions(subjectOf(mail.targets())));

  // Le registre n'est pas réactif : relecture à chaque modification.
  const watch = new RegistryWatch(() => registry);

  function describe(id: string, label?: string) {
    void watch.version;
    return describeCommand(registry, id, context, label);
  }

  const bulk = bulkActions();

  const ROW_HEIGHT = 60;
  let scroller: HTMLElement | null = $state(null);
  const list = new VirtualList(() => scroller, ROW_HEIGHT);

  $effect(() => list.mount());
  $effect(() => list.setCount(mail.conversations.length));
  $effect(() => list.scrollToIndex(mail.selectedIndex));
  $effect(() => {
    const last = list.items.at(-1);
    if (last !== undefined && mail.hasMore && last.index >= mail.conversations.length - 20) {
      void mail.loadMore();
    }
  });

  function senders(index: number): string {
    const c = mail.conversations[index];
    if (c === undefined || c.senders.length === 0) return m.common_unknown_sender();
    return c.senders.map((s) => personName(s)).join(", ");
  }

  /** Bouton de la souris enfoncé sur une ligne : début possible d'un
   * glisser (pas depuis la case à cocher ni les boutons de la ligne). */
  function press(e: PointerEvent, index: number): void {
    const c = mail.conversations[index];
    if (drag === undefined || c === undefined || e.button !== 0) return;
    if (e.pointerType !== "mouse" && e.pointerType !== "pen") return;
    if (e.target instanceof Element && e.target.closest("input, button") !== null) return;
    const payload = dragPayload(c, mail.marked, mail.conversations);
    if (payload !== null) drag.press(payload, e.clientX, e.clientY);
  }

  function choose(index: number): void {
    // Clic qui termine un glisser : la conversation n'est pas ouverte.
    if (drag?.suppressClick()) return;
    mail.select(index);
    void mail.open();
  }
</script>

<div class="conversations">
  <header>
    <h2>{title}</h2>
    {#if onSearch}
      <button
        class="btn search-button"
        type="button"
        aria-label={m.search_label()}
        title={describe("search.open", m.search_label())?.tooltip ?? m.search_label()}
        onclick={onSearch}
      >
        <SearchIcon size={16} aria-hidden="true" />
        <span>{m.search_button()}</span>
      </button>
    {/if}
  </header>
  {#if mail.marked.length > 0}
    {@const clear = describe("selection.clear")}
    <section class="bulk" aria-label={m.list_selection()}>
      <div class="bulk-head">
        <span class="marked-count" role="status"
          >{m.list_marked({ count: mail.marked.length })}</span
        >
        <button
          class="btn btn-ghost"
          type="button"
          title={clear?.tooltip}
          onclick={() => void registry.execute("selection.clear", context)}
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
        label={m.list_bulk_actions()}
      />
    </section>
  {/if}
  {#if mail.error}
    <p class="error" role="alert">{mail.error}</p>
  {/if}
  {#if mail.conversations.length === 0}
    <p class="empty">{m.list_empty()}</p>
  {/if}
  <ContextMenu.Root>
    <ContextMenu.Trigger>
      {#snippet child({ props })}
        <div
          {...props}
          class="scroller"
          bind:this={scroller}
          role="listbox"
          aria-label={m.list_conversations()}
          aria-multiselectable={mail.marked.length > 0 ? "true" : undefined}
          tabindex="0"
          aria-activedescendant={mail.selected ? `conversation-${mail.selected.id}` : undefined}
        >
          <div class="spacer" style:height="{list.totalSize}px">
            {#each list.items as row (row.key)}
              {@const c = mail.conversations[row.index]}
              {#if c !== undefined}
                <div
                  id="conversation-{c.id}"
                  class="row"
                  class:selected={row.index === mail.selectedIndex}
                  class:marked={mail.marked.includes(c.id)}
                  class:unread={c.unread > 0}
                  class:dragged={drag?.payload?.ids.includes(c.id) ?? false}
                  role="option"
                  aria-selected={mail.marked.length > 0
                    ? mail.marked.includes(c.id)
                    : row.index === mail.selectedIndex}
                  tabindex="-1"
                  style:height="{ROW_HEIGHT}px"
                  style:transform="translateY({row.start}px)"
                  onpointerdown={(e) => press(e, row.index)}
                  onclick={() => choose(row.index)}
                  oncontextmenu={() => mail.pointAt(row.index)}
                  onkeydown={(e) => {
                    if (e.key === " ") {
                      e.preventDefault();
                      choose(row.index);
                    }
                  }}
                >
                  <input
                    class="check"
                    type="checkbox"
                    tabindex="-1"
                    checked={mail.marked.includes(c.id)}
                    aria-label={m.list_select_row({
                      subject: c.subject !== "" ? c.subject : m.common_no_subject(),
                    })}
                    title={m.list_mark()}
                    onclick={(e) => {
                      // Cocher n'ouvre pas la conversation.
                      e.stopPropagation();
                      mail.toggleMarkAt(row.index);
                    }}
                  />
                  <div class="content">
                    <div class="line">
                      <span class="senders">{senders(row.index)}</span>
                      {#if c.messages > 1}<span class="count">{c.messages}</span>{/if}
                      <span class="date">{shortDate(c.date)}</span>
                    </div>
                    <div class="row-actions">
                      <ActionButtons
                        actions={rowActions(c)}
                        {registry}
                        {context}
                        label={m.list_row_actions()}
                        compact
                        onBefore={() => mail.pointAt(row.index)}
                      />
                    </div>
                    <div class="line">
                      <span class="subject"
                        >{c.subject !== "" ? c.subject : m.common_no_subject()}</span
                      >
                      {#if c.has_attachments}<PaperclipIcon
                          size={14}
                          aria-label={m.list_attachment()}
                        />{/if}
                      {#if c.flagged}<FlagIcon size={14} aria-label={m.list_flagged()} />{/if}
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
      <ContextMenu.Content class="menu" aria-label={m.list_context_menu()}>
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
  .conversations {
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

  .search-button {
    margin-left: auto;
    align-self: center;
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

  h2 {
    margin: 0;
    font-size: var(--font-size-lg);
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

  .row:hover {
    background: var(--color-hover);
  }

  .row.selected {
    background: var(--color-selected);
  }

  /* Actions au survol (souris) : par-dessus la date, à droite. */
  .row-actions {
    position: absolute;
    top: var(--space-1);
    right: var(--space-2);
    display: none;
    border-radius: var(--radius);
    background: var(--color-surface-raised);
  }

  .row:hover .row-actions {
    display: block;
  }

  /* Conversations en cours de glisser. */
  .row.dragged {
    opacity: 0.5;
  }

  .row.marked {
    box-shadow: inset 3px 0 0 var(--color-accent);
  }

  .marked-count {
    font-weight: var(--font-weight-medium);
  }

  .line {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .senders,
  .subject {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .subject {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .unread .senders,
  .unread .subject {
    color: var(--color-text);
    font-weight: var(--font-weight-bold);
  }

  .count {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .date {
    flex: none;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
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
