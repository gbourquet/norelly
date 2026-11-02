<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Liste virtualisée des conversations : seules les lignes visibles sont
  // dans le DOM, quelle que soit la taille de la boîte.
  import PaperclipIcon from "@lucide/svelte/icons/paperclip";
  import FlagIcon from "@lucide/svelte/icons/flag";
  import CheckIcon from "@lucide/svelte/icons/check";
  import type { MailState } from "../lib/mail.svelte";
  import { VirtualList } from "../lib/virtual-list.svelte";
  import { personName, shortDate } from "../lib/format";

  interface Props {
    mail: MailState;
    title: string;
  }

  let { mail, title }: Props = $props();

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

  function choose(index: number): void {
    mail.select(index);
    void mail.open();
  }
</script>

<div class="conversations">
  <header>
    <h2>{title}</h2>
    {#if mail.marked.length > 0}
      <span class="marked-count">{m.list_marked({ count: mail.marked.length })}</span>
    {/if}
  </header>
  {#if mail.error}
    <p class="error" role="alert">{mail.error}</p>
  {/if}
  {#if mail.conversations.length === 0}
    <p class="empty">{m.list_empty()}</p>
  {/if}
  <div
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
            role="option"
            aria-selected={mail.marked.length > 0
              ? mail.marked.includes(c.id)
              : row.index === mail.selectedIndex}
            tabindex="-1"
            style:height="{ROW_HEIGHT}px"
            style:transform="translateY({row.start}px)"
            onclick={() => choose(row.index)}
            onkeydown={(e) => {
              if (e.key === " ") {
                e.preventDefault();
                choose(row.index);
              }
            }}
          >
            <div class="line">
              {#if mail.marked.includes(c.id)}<CheckIcon
                  size={14}
                  aria-label={m.list_mark()}
                />{/if}
              <span class="senders">{senders(row.index)}</span>
              {#if c.messages > 1}<span class="count">{c.messages}</span>{/if}
              <span class="date">{shortDate(c.date)}</span>
            </div>
            <div class="line">
              <span class="subject">{c.subject !== "" ? c.subject : m.common_no_subject()}</span>
              {#if c.has_attachments}<PaperclipIcon
                  size={14}
                  aria-label={m.list_attachment()}
                />{/if}
              {#if c.flagged}<FlagIcon size={14} aria-label={m.list_flagged()} />{/if}
            </div>
          </div>
        {/if}
      {/each}
    </div>
  </div>
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
    align-items: baseline;
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

  .scroller {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    outline: none;
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
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    padding: 0 var(--space-4);
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

  .marked-count {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
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
