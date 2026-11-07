<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Confirmation de suppression d'un dossier et de ses messages.
  import Modal from "./Modal.svelte";
  import type { AccountDto, FolderDto } from "../lib/ipc/api";
  import type { DeleteFolderState } from "../lib/folder-edit.svelte";

  interface Props {
    open: boolean;
    account: AccountDto | null;
    folder: FolderDto | null;
    form: DeleteFolderState;
    onDeleted: (folder: FolderDto) => void;
    onClose: () => void;
  }

  let { open, account, folder, form, onDeleted, onClose }: Props = $props();

  async function confirm(): Promise<void> {
    if (account === null || folder === null) return;
    const target = folder;
    if (await form.submit(account.id, target)) onDeleted(target);
  }
</script>

<Modal
  {open}
  title={m.delete_folder_title()}
  description={account?.address}
  {onClose}
  locked={form.busy}
>
  {#if folder}
    <!-- Sans compte ni « vide » : le nombre connu localement peut être faux
         (dossier pas encore synchronisé). -->
    <p>{m.delete_folder_explanation({ folder: folder.leaf })}</p>
    <p>{m.delete_folder_irreversible()}</p>
  {/if}
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
      disabled={form.busy || account === null || folder === null}
    >
      {form.busy ? m.delete_folder_busy() : m.delete_folder_confirm()}
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
