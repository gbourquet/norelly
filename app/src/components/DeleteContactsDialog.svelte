<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Confirmation de suppression d'un ou de plusieurs contacts du carnet.
  import Modal from "./Modal.svelte";
  import type { DeleteContactsState } from "../lib/contacts.svelte";

  interface Props {
    open: boolean;
    /** Contacts visés. */
    ids: number[];
    /** Nom du contact visé s'il est seul (sinon `null`). */
    name: string | null;
    form: DeleteContactsState;
    onDeleted: (ids: number[], count: number) => void;
    onClose: () => void;
  }

  let { open, ids, name, form, onDeleted, onClose }: Props = $props();

  async function confirm(): Promise<void> {
    const targets = [...ids];
    const count = await form.submit(targets);
    if (count !== null) onDeleted(targets, count);
  }
</script>

<Modal
  {open}
  title={ids.length > 1 ? m.delete_contacts_title_many() : m.delete_contacts_title()}
  {onClose}
  locked={form.busy}
>
  <p>
    {name !== null && ids.length === 1
      ? m.delete_contacts_one({ name })
      : m.delete_contacts_many({ count: ids.length })}
  </p>
  <p>{m.delete_contacts_mail_kept()}</p>
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
      disabled={form.busy || ids.length === 0}
    >
      {form.busy ? m.delete_contacts_busy() : m.delete_contacts_confirm()}
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
