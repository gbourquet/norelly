<script lang="ts">
  // Brouillons enregistrés d'un compte : un clic les rouvre dans l'éditeur.
  import Modal from "./Modal.svelte";
  import type { DraftSummaryDto } from "../lib/ipc/api";
  import { shortDate } from "../lib/format";
  import { m } from "../lib/paraglide/messages.js";

  interface Props {
    open: boolean;
    drafts: DraftSummaryDto[];
    onOpen: (id: number) => void;
    onClose: () => void;
  }

  let { open, drafts, onOpen, onClose }: Props = $props();
</script>

<Modal {open} title={m.drafts_title()} width="560px" {onClose}>
  {#if drafts.length === 0}
    <p>{m.drafts_empty()}</p>
  {:else}
    <ul class="drafts">
      {#each drafts as d (d.id)}
        <li>
          <button class="draft" type="button" onclick={() => onOpen(d.id)}>
            <strong>{d.subject || m.outbox_no_subject()}</strong>
            <span class="muted">{m.drafts_to({ recipients: d.to.join(", ") })}</span>
            <span class="muted date">{shortDate(d.updated_at)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</Modal>

<style>
  .drafts {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .draft {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px var(--space-3);
    width: 100%;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius);
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .draft:hover {
    background: var(--color-hover);
  }

  .muted {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .date {
    grid-row: 1;
    grid-column: 2;
  }
</style>
