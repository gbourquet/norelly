<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Renommage d'un dossier : nouveau nom, à la même place. Le nom complet
  // est décidé par Rust.
  import Modal from "./Modal.svelte";
  import type { AccountDto, FolderDto } from "../lib/ipc/api";
  import type { RenamedFolder, RenameFolderState } from "../lib/folder-edit.svelte";

  interface Props {
    open: boolean;
    account: AccountDto | null;
    folder: FolderDto | null;
    form: RenameFolderState;
    onRenamed: (renamed: RenamedFolder) => void;
    onClose: () => void;
  }

  let { open, account, folder, form, onRenamed, onClose }: Props = $props();

  async function submit(): Promise<void> {
    if (account === null || folder === null) return;
    const renamed = await form.submit(account.id, folder);
    if (renamed !== null) onRenamed(renamed);
  }
</script>

<Modal
  {open}
  title={m.rename_folder_title()}
  description={account?.address}
  {onClose}
  locked={form.busy}
>
  <form
    class="rename-folder"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    <label class="field">
      <span>{m.rename_folder_name()}</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input" type="text" bind:value={form.name} autofocus disabled={form.busy} />
    </label>
    {#if form.error}
      <p class="error" role="alert">{form.error}</p>
    {/if}
    <div class="actions">
      <button class="btn" type="button" onclick={onClose} disabled={form.busy}
        >{m.common_cancel()}</button
      >
      <button
        class="btn btn-primary"
        type="submit"
        disabled={!form.canSubmit || account === null || folder === null}
      >
        {form.busy ? m.rename_folder_busy() : m.rename_folder_confirm()}
      </button>
    </div>
  </form>
</Modal>

<style>
  .rename-folder {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .error {
    margin: 0;
    color: var(--color-danger);
  }
</style>
