<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Ajout d'un ou de plusieurs contacts à une liste de diffusion : choix de
  // la liste parmi celles du carnet. Les contacts déjà membres sont ignorés
  // par Rust.
  import UsersIcon from "@lucide/svelte/icons/users";
  import Modal from "./Modal.svelte";
  import type { ContactList } from "../lib/ipc/api";
  import type { AddToListState } from "../lib/contact-lists.svelte";

  interface Props {
    open: boolean;
    /** Contacts à ajouter. */
    ids: number[];
    /** Nom du contact s'il est seul (sinon `null`). */
    name: string | null;
    form: AddToListState;
    onAdded: (list: ContactList) => void;
    onClose: () => void;
  }

  let { open, ids, name, form, onAdded, onClose }: Props = $props();

  async function submit(): Promise<void> {
    const list = await form.submit([...ids]);
    if (list !== null) onAdded(list);
  }
</script>

<Modal {open} title={m.add_to_list_title()} width="440px" {onClose} locked={form.busy}>
  <form
    class="add-form"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    <p>
      {name !== null && ids.length === 1
        ? m.add_to_list_intro_one({ name })
        : m.add_to_list_intro_many({ count: ids.length })}
    </p>
    {#if !form.loading && form.lists.length === 0 && form.error === null}
      <p class="muted">{m.add_to_list_none()}</p>
    {:else}
      <fieldset>
        <legend class="sr-only">{m.add_to_list_choose()}</legend>
        {#each form.lists as l (l.id)}
          <label class="choice" class:chosen={form.chosen === l.id}>
            <input
              type="radio"
              name="list"
              value={l.id}
              checked={form.chosen === l.id}
              onchange={() => (form.chosen = l.id)}
              disabled={form.busy}
            />
            <UsersIcon size={16} aria-hidden="true" />
            <span class="name">{l.name}</span>
            <span class="muted">{m.lists_members_count({ count: l.members })}</span>
          </label>
        {/each}
      </fieldset>
    {/if}
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
        disabled={form.busy || form.chosen === null || ids.length === 0}
      >
        {form.busy ? m.add_to_list_busy() : m.add_to_list_submit()}
      </button>
    </div>
  </form>
</Modal>

<style>
  .add-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  p {
    margin: 0;
  }

  fieldset {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 16rem;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    border: none;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  .choice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .choice:hover {
    background: var(--color-hover);
  }

  .choice.chosen {
    border-color: var(--color-accent);
    background: var(--color-selected);
  }

  .choice input {
    margin: 0;
    accent-color: var(--color-accent);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .muted {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .error {
    color: var(--color-danger);
  }
</style>
