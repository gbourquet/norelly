<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Création d'un dossier : nom et emplacement (premier niveau ou dans un
  // dossier existant du compte). Le nom complet est décidé par Rust.
  import Modal from "./Modal.svelte";
  import type { AccountDto, FolderDto } from "../lib/ipc/api";
  import { parentChoices, type NewFolderState } from "../lib/new-folder.svelte";

  interface Props {
    open: boolean;
    account: AccountDto | null;
    folders: FolderDto[];
    form: NewFolderState;
    onCreated: (name: string) => void;
    onClose: () => void;
  }

  let { open, account, folders, form, onCreated, onClose }: Props = $props();

  const choices = $derived(parentChoices(folders, m.new_folder_top_level()));

  /** Retrait visuel d'un emplacement dans la liste (espaces insécables). */
  function indent(depth: number): string {
    return "   ".repeat(Math.min(depth, 6));
  }

  async function submit(): Promise<void> {
    if (account === null) return;
    const created = await form.submit(account.id);
    if (created !== null) onCreated(created);
  }
</script>

<Modal
  {open}
  title={m.new_folder_title()}
  description={account?.address}
  {onClose}
  locked={form.busy}
>
  <form
    class="new-folder"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    <label class="field">
      <span>{m.new_folder_name()}</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input" type="text" bind:value={form.name} autofocus disabled={form.busy} />
    </label>
    <label class="field">
      <span>{m.new_folder_parent()}</span>
      <select bind:value={form.parent} disabled={form.busy}>
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
      <button class="btn btn-primary" type="submit" disabled={!form.canSubmit || account === null}>
        {form.busy ? m.new_folder_creating() : m.new_folder_create()}
      </button>
    </div>
  </form>
</Modal>

<style>
  .new-folder {
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
