<script lang="ts">
  import { unixOfLocal } from "../lib/outbox";
  // Fenêtre de rédaction : destinataires, objet, éditeur, pièces jointes.
  // Ctrl+Entrée envoie ; Échap ferme en gardant le brouillon.
  import PaperclipIcon from "@lucide/svelte/icons/paperclip";
  import SendIcon from "@lucide/svelte/icons/send";
  import TrashIcon from "@lucide/svelte/icons/trash-2";
  import XIcon from "@lucide/svelte/icons/x";
  import Modal from "./Modal.svelte";
  import Editor from "./Editor.svelte";
  import RecipientInput from "./RecipientInput.svelte";
  import type { ComposeState, RecipientField } from "../lib/compose.svelte";
  import { fileSize } from "../lib/format";
  import { m } from "../lib/paraglide/messages.js";

  interface Props {
    open: boolean;
    title: string;
    compose: ComposeState;
    onClose: () => void;
    /** `scheduled` : envoi différé plutôt qu'immédiat. */
    onSent: (scheduled: boolean) => void;
  }

  let { open, title, compose, onClose, onSent }: Props = $props();

  let showCopies = $state(false);
  const draft = $derived(compose.draft);

  const fields: { field: RecipientField; label: () => string }[] = [
    { field: "to", label: m.compose_to },
    { field: "cc", label: m.compose_cc },
    { field: "bcc", label: m.compose_bcc },
  ];

  /** Date et heure d'un envoi différé (`datetime-local`). */
  let later = $state("");

  async function send(at: number | null = null): Promise<void> {
    if (await compose.send(at)) {
      later = "";
      onSent(at !== null);
    }
  }

  function onKeyDown(e: KeyboardEvent): void {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      e.stopPropagation();
      void send();
    }
  }
</script>

<Modal {open} {title} width="760px" locked={compose.busy} {onClose}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="compose" onkeydown={onKeyDown}>
    {#if draft !== null}
      {#each fields as { field, label } (field)}
        {#if field === "to" || showCopies || draft[field].length > 0}
          <RecipientInput
            label={label()}
            recipients={draft[field]}
            suggestions={compose.suggestions}
            onAdd={(raw) => compose.addRecipient(field, raw)}
            onRemove={(i) => compose.removeRecipient(field, i)}
            onQuery={(text) => compose.query(text)}
          />
        {/if}
      {/each}
      {#if !showCopies && draft.cc.length === 0 && draft.bcc.length === 0}
        <button class="link-button" type="button" onclick={() => (showCopies = true)}
          >{m.compose_show_cc()}</button
        >
      {/if}
      <div class="subject">
        <label for="compose-subject">{m.compose_subject()}</label>
        <input
          id="compose-subject"
          type="text"
          value={draft.subject}
          oninput={(e) => compose.edit({ subject: e.currentTarget.value })}
        />
      </div>
      {#key draft.account}
        <Editor
          html={draft.html}
          label={m.compose_body()}
          onChange={(html) => compose.edit({ html })}
        />
      {/key}
      {#if compose.attachments.length > 0}
        <ul class="attachments">
          {#each compose.attachments as a, i (a.blob + i)}
            <li>
              <PaperclipIcon size={14} aria-hidden="true" />
              <span>{a.file_name}</span>
              <span class="size">{fileSize(a.size)}</span>
              <button
                type="button"
                aria-label={m.compose_remove_attachment({ name: a.file_name })}
                onclick={() => compose.removeAttachment(i)}><XIcon size={12} /></button
              >
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
    {#if compose.error}
      <p class="error" role="alert">{compose.error}</p>
    {/if}
    <div class="actions">
      <button
        class="btn"
        type="button"
        disabled={draft === null}
        onclick={() => void compose.pickAttachments(m.compose_attach_title())}
        ><PaperclipIcon size={16} aria-hidden="true" /> {m.compose_attach()}</button
      >
      <button
        class="btn"
        type="button"
        disabled={draft === null || compose.busy}
        onclick={async () => {
          await compose.discard();
          onClose();
        }}><TrashIcon size={16} aria-hidden="true" /> {m.compose_discard()}</button
      >
      <div class="schedule">
        <label class="later">
          <span>{m.compose_later()}</span>
          <input type="datetime-local" bind:value={later} disabled={draft === null} />
        </label>
        <button
          class="btn"
          type="button"
          disabled={draft === null || compose.busy || unixOfLocal(later) === null}
          onclick={() => void send(unixOfLocal(later))}>{m.compose_schedule()}</button
        >
      </div>
      <span class="hint">{m.compose_send_hint()}</span>
      <button
        class="btn btn-primary"
        type="button"
        disabled={draft === null || compose.busy}
        onclick={() => void send()}
        ><SendIcon size={16} aria-hidden="true" /> {m.compose_send()}</button
      >
    </div>
  </div>
</Modal>

<style>
  .schedule {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
  }

  .later {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
  }

  .compose {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .subject {
    display: grid;
    grid-template-columns: 4rem 1fr;
    align-items: center;
    gap: var(--space-2);
  }

  .subject input {
    padding: var(--space-1) var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    background: transparent;
    color: inherit;
  }

  .link-button {
    align-self: flex-start;
    margin-left: calc(4rem + var(--space-2));
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
    cursor: pointer;
  }

  .attachments {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .attachments li {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .attachments button {
    display: inline-flex;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  .size,
  .hint {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .error {
    margin: 0;
    color: var(--color-danger, inherit);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  /* Libellés sur une ligne : la rangée passe à la ligne entre deux
     éléments plutôt qu'au milieu d'un bouton. */
  .actions .btn,
  .actions .hint,
  .later span {
    flex: none;
    white-space: nowrap;
  }

  .actions .hint {
    margin-left: auto;
  }
</style>
