<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Déplacement d'un dossier : nouvel emplacement (premier niveau ou dans
  // un autre dossier du compte), avec ses sous-dossiers. Le nom complet est
  // décidé par Rust.
  import Modal from "./Modal.svelte";
  import type { AccountDto, FolderDto } from "../lib/ipc/api";
  import {
    destinationChoices,
    type MoveFolderState,
    type RenamedFolder,
  } from "../lib/folder-edit.svelte";

  interface Props {
    open: boolean;
    account: AccountDto | null;
    folder: FolderDto | null;
    folders: FolderDto[];
    form: MoveFolderState;
    onMoved: (moved: RenamedFolder) => void;
    onClose: () => void;
  }

  let { open, account, folder, folders, form, onMoved, onClose }: Props = $props();

  const choices = $derived(
    folder === null ? [] : destinationChoices(folders, folder, m.new_folder_top_level()),
  );

  /** Retrait visuel d'un emplacement dans la liste (espaces insécables). */
  function indent(depth: number): string {
    return "   ".repeat(Math.min(depth, 6));
  }

  async function submit(): Promise<void> {
    if (account === null || folder === null) return;
    const moved = await form.submit(account.id, folder);
    if (moved !== null) onMoved(moved);
  }
</script>

<Modal
  {open}
  title={m.move_folder_title()}
  description={account?.address}
  {onClose}
  locked={form.busy}
>
  <form
    class="move-folder"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    {#if folder}
      <p class="explanation">{m.move_folder_explanation({ folder: folder.leaf })}</p>
    {/if}
    <label class="field">
      <span>{m.move_folder_parent()}</span>
      <!-- svelte-ignore a11y_autofocus -->
      <select bind:value={form.parent} disabled={form.busy} autofocus>
        {#each choices as choice (choice.id ?? 0)}
          <option value={choice.id}>{indent(choice.depth)}{choice.label}</option>
        {/each}
      </select>
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
        {form.busy ? m.move_folder_busy() : m.move_folder_confirm()}
      </button>
    </div>
  </form>
</Modal>

<style>
  .move-folder {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .explanation {
    margin: 0;
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
