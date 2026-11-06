<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Vue d'une conversation : messages empilés du plus ancien au plus
  // récent ; le dernier et les non lus sont dépliés, les autres se
  // déplient d'un clic.
  import type { Snippet } from "svelte";
  import ArrowLeftIcon from "@lucide/svelte/icons/arrow-left";
  import type { Api, ConversationDto, ConversationMessage } from "../lib/ipc/api";
  import type { Shortcut } from "../lib/commands/shortcut";
  import MessageBody from "./MessageBody.svelte";
  import { longDate, personName } from "../lib/format";

  interface Props {
    conversation: ConversationDto;
    messages: ConversationMessage[];
    api: Api;
    onBack: () => void;
    onKey: (shortcut: Shortcut) => void;
    /** Barre d'actions (sous le titre). */
    toolbar?: Snippet;
  }

  let { conversation, messages, api, onBack, onKey, toolbar }: Props = $props();

  let expanded = $state<number[]>([]);

  $effect(() => {
    const last = messages.at(-1);
    expanded = messages.filter((x) => x.unread || x.id === last?.id).map((x) => x.id);
  });

  function toggle(id: number): void {
    expanded = expanded.includes(id) ? expanded.filter((e) => e !== id) : [...expanded, id];
  }

  function recipients(msg: ConversationMessage): string {
    return [...msg.to, ...msg.cc].map((p) => personName(p)).join(", ");
  }
</script>

<article class="conversation" aria-label={m.conversation_label()}>
  <header>
    <button
      class="btn btn-ghost icon"
      type="button"
      title={m.conversation_back_title()}
      aria-label={m.conversation_back()}
      onclick={onBack}
    >
      <ArrowLeftIcon size={16} aria-hidden="true" />
    </button>
    <h1>{conversation.subject !== "" ? conversation.subject : m.common_no_subject()}</h1>
    {#if toolbar}
      <div class="toolbar">{@render toolbar()}</div>
    {/if}
  </header>
  <ol class="messages">
    {#each messages as msg (msg.id)}
      {@const open = expanded.includes(msg.id)}
      <li class="message" class:open>
        <button class="summary" type="button" aria-expanded={open} onclick={() => toggle(msg.id)}>
          <span class="from" class:unread={msg.unread}>{personName(msg.from)}</span>
          {#if msg.from && msg.from.name !== ""}<span class="address"
              >&lt;{msg.from.address}&gt;</span
            >{/if}
          <span class="date">{longDate(msg.date)}</span>
        </button>
        {#if open}
          {#if recipients(msg) !== ""}
            <p class="to">{m.conversation_to({ recipients: recipients(msg) })}</p>
          {/if}
          <MessageBody message={msg.id} {api} {onKey} />
        {/if}
      </li>
    {/each}
  </ol>
</article>

<style>
  .conversation {
    display: flex;
    flex-direction: column;
    min-height: 100%;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-bottom: var(--border-width) solid var(--color-border);
  }

  .toolbar {
    flex: 1 0 100%;
    min-width: 0;
  }

  h1 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-medium);
    overflow-wrap: anywhere;
  }

  .icon {
    padding: var(--space-1);
    min-height: 0;
  }

  .messages {
    margin: 0;
    padding: var(--space-3);
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .message {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    padding: var(--space-2) var(--space-3);
    background: var(--color-surface-raised);
  }

  .summary {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-1) 0;
    border: none;
    background: transparent;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }

  .from.unread {
    font-weight: var(--font-weight-bold);
  }

  .address,
  .to,
  .date {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .date {
    margin-left: auto;
  }

  .to {
    margin: 0 0 var(--space-2);
  }
</style>
