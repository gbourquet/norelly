<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Vue d'une conversation : messages empilés du plus ancien au plus
  // récent ; le dernier et les non lus sont dépliés, les autres se
  // déplient d'un clic.
  import type { Snippet } from "svelte";
  import ArrowLeftIcon from "@lucide/svelte/icons/arrow-left";
  import type { Api, ConversationDto, ConversationMessage, CorrespondentDto } from "../lib/ipc/api";
  import type { Shortcut } from "../lib/commands/shortcut";
  import MessageBody from "./MessageBody.svelte";
  import { longDate, personName } from "../lib/format";

  interface Props {
    conversation: ConversationDto;
    messages: ConversationMessage[];
    api: Api;
    onBack: () => void;
    onKey: (shortcut: Shortcut) => void;
    /** Infobulle du bouton retour (avec son raccourci). */
    backTitle: string;
    /** Barre d'actions (sous le titre). */
    toolbar?: Snippet;
    /** Nom d'une personne (expéditeur en gras si non lu, destinataire),
     * avec son menu ; sinon, le nom seul. */
    person?: Snippet<[CorrespondentDto, boolean]>;
  }

  let { conversation, messages, api, onBack, onKey, backTitle, toolbar, person }: Props = $props();

  let expanded = $state<number[]>([]);

  $effect(() => {
    const last = messages.at(-1);
    expanded = messages.filter((x) => x.unread || x.id === last?.id).map((x) => x.id);
  });

  function toggle(id: number): void {
    expanded = expanded.includes(id) ? expanded.filter((e) => e !== id) : [...expanded, id];
  }

  function recipients(msg: ConversationMessage): CorrespondentDto[] {
    return [...msg.to, ...msg.cc];
  }
</script>

<article class="conversation" aria-label={m.conversation_label()}>
  <header>
    <button
      class="btn btn-ghost icon"
      type="button"
      title={backTitle}
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
        <div class="summary">
          <!-- Toute la ligne déplie le message ; le nom de l'expéditeur
               garde son propre menu par-dessus. -->
          <button
            class="toggle"
            type="button"
            aria-expanded={open}
            aria-label={m.conversation_message_toggle({
              name: personName(msg.from),
              date: longDate(msg.date),
            })}
            onclick={() => toggle(msg.id)}
          ></button>
          {#if msg.from && person}
            <span class="who">{@render person(msg.from, msg.unread)}</span>
          {:else}
            <span class="from" class:unread={msg.unread}>{personName(msg.from)}</span>
          {/if}
          {#if msg.from && msg.from.name !== ""}<span class="address"
              >&lt;{msg.from.address}&gt;</span
            >{/if}
          <span class="date">{longDate(msg.date)}</span>
        </div>
        {#if open}
          {#if recipients(msg).length > 0}
            <p class="to">
              <span>{m.conversation_to_label()}</span>
              {#each recipients(msg) as p, i (i)}
                {#if i > 0}<span aria-hidden="true">, </span>{/if}
                {#if person}{@render person(p, false)}{:else}<span>{personName(p)}</span>{/if}
              {/each}
            </p>
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
    position: relative;
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-1) 0;
  }

  .toggle {
    position: absolute;
    inset: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    cursor: pointer;
  }

  .summary > :not(.toggle) {
    pointer-events: none;
  }

  .summary > .who {
    position: relative;
    min-width: 0;
    pointer-events: auto;
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
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0 var(--space-1);
    margin: 0 0 var(--space-2);
  }
</style>
