<script lang="ts">
  // Boîte d'envoi d'un compte : messages en attente, en cours ou en échec,
  // avec « réessayer » et « annuler l'envoi ».
  import Modal from "./Modal.svelte";
  import type { OutboxItemDto } from "../lib/ipc/api";
  import { isScheduled, stateLabel } from "../lib/outbox";
  import { m } from "../lib/paraglide/messages.js";

  interface Props {
    open: boolean;
    items: OutboxItemDto[];
    onRetry: (id: number) => void;
    onCancel: (id: number) => void;
    /** Reprend un envoi programmé pour le modifier. */
    onEdit: (id: number) => void;
    onClose: () => void;
  }

  let { open, items, onRetry, onCancel, onEdit, onClose }: Props = $props();
</script>

<Modal {open} title={m.outbox_title()} width="560px" {onClose}>
  {#if items.length === 0}
    <p>{m.outbox_empty()}</p>
  {:else}
    <ul class="outbox">
      {#each items as item (item.id)}
        <li>
          <div class="what">
            <strong>{item.subject || m.outbox_no_subject()}</strong>
            <span class="muted">{item.recipients.join(", ")}</span>
            <span class="muted">{stateLabel(item.state)}</span>
          </div>
          {#if item.state.state === "failed"}
            <button class="btn" type="button" onclick={() => onRetry(item.id)}
              >{m.outbox_retry()}</button
            >
          {/if}
          {#if isScheduled(item.state)}
            <button class="btn" type="button" onclick={() => onEdit(item.id)}
              >{m.outbox_edit()}</button
            >
          {/if}
          {#if item.state.state === "failed" || item.state.state === "queued"}
            <button class="btn" type="button" onclick={() => onCancel(item.id)}
              >{m.outbox_cancel()}</button
            >
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</Modal>

<style>
  .outbox {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .what {
    display: flex;
    flex: 1;
    flex-direction: column;
  }

  .muted {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }
</style>
