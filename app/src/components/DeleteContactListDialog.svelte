<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Confirmation de suppression d'une liste de diffusion : ses membres
  // restent dans le carnet.
  import Modal from "./Modal.svelte";
  import type { DeleteContactListState } from "../lib/contact-lists.svelte";

  interface Props {
    open: boolean;
    /** Liste visée. */
    id: number | null;
    name: string;
    form: DeleteContactListState;
    onDeleted: (id: number) => void;
    onClose: () => void;
  }

  let { open, id, name, form, onDeleted, onClose }: Props = $props();

  async function confirm(): Promise<void> {
    if (id === null) return;
    const target = id;
    if ((await form.submit(target)) !== null) onDeleted(target);
  }
</script>

<Modal {open} title={m.delete_list_title()} {onClose} locked={form.busy}>
  <p>{m.delete_list_confirm_text({ name })}</p>
  <p>{m.delete_list_contacts_kept()}</p>
  {#if form.error}
    <p class="error" role="alert">{form.error}</p>
  {/if}
  <div class="actions">
    <button class="btn" type="button" onclick={onClose} disabled={form.busy}
      >{m.common_cancel()}</button
    >
    <button
      class="btn btn-danger"
      type="button"
      onclick={() => void confirm()}
      disabled={form.busy || id === null}
    >
      {form.busy ? m.delete_list_busy() : m.delete_list_confirm()}
    </button>
  </div>
</Modal>

<style>
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-4);
  }

  .error {
    color: var(--color-danger);
  }
</style>
