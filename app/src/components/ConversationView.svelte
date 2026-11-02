<script lang="ts">
  // Vue d'une conversation : messages empilés du plus ancien au plus
  // récent ; le dernier et les non lus sont dépliés, les autres se
  // déplient d'un clic.
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
  }

  let { conversation, messages, api, onBack, onKey }: Props = $props();

  let expanded = $state<number[]>([]);

  $effect(() => {
    const last = messages.at(-1);
    expanded = messages.filter((m) => m.unread || m.id === last?.id).map((m) => m.id);
  });

  function toggle(id: number): void {
    expanded = expanded.includes(id) ? expanded.filter((e) => e !== id) : [...expanded, id];
  }

  function recipients(m: ConversationMessage): string {
    return [...m.to, ...m.cc].map((p) => personName(p)).join(", ");
  }
</script>

<article class="conversation" aria-label="Conversation">
  <header>
    <button
      class="btn btn-ghost icon"
      type="button"
      title="Retour à la liste (Échap)"
      aria-label="Retour à la liste"
      onclick={onBack}
    >
      <ArrowLeftIcon size={16} aria-hidden="true" />
    </button>
    <h1>{conversation.subject !== "" ? conversation.subject : "(sans objet)"}</h1>
  </header>
  <ol class="messages">
    {#each messages as m (m.id)}
      {@const open = expanded.includes(m.id)}
      <li class="message" class:open>
        <button class="summary" type="button" aria-expanded={open} onclick={() => toggle(m.id)}>
          <span class="from" class:unread={m.unread}>{personName(m.from)}</span>
          {#if m.from && m.from.name !== ""}<span class="address">&lt;{m.from.address}&gt;</span
            >{/if}
          <span class="date">{longDate(m.date)}</span>
        </button>
        {#if open}
          {#if recipients(m) !== ""}
            <p class="to">À : {recipients(m)}</p>
          {/if}
          <MessageBody message={m.id} {api} {onKey} />
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
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-bottom: var(--border-width) solid var(--color-border);
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
