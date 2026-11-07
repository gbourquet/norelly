<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Création ou modification d'une liste de diffusion : nom, description,
  // envoi en copie cachée, membres choisis parmi les contacts (recherche
  // par nom ou adresse). La liste est revalidée par Rust ; ses refus
  // s'affichent au-dessus des boutons.
  import PlusIcon from "@lucide/svelte/icons/plus";
  import SearchIcon from "@lucide/svelte/icons/search";
  import XIcon from "@lucide/svelte/icons/x";
  import CheckIcon from "@lucide/svelte/icons/check";
  import Modal from "./Modal.svelte";
  import type { ContactList } from "../lib/ipc/api";
  import type { ContactListFormState } from "../lib/contact-lists.svelte";

  interface Props {
    open: boolean;
    form: ContactListFormState;
    onSaved: (list: ContactList) => void;
    onClose: () => void;
  }

  let { open, form, onSaved, onClose }: Props = $props();

  let query = $state("");
  let timer: ReturnType<typeof setTimeout> | null = null;

  // Formulaire rouvert : recherche vidée.
  $effect(() => {
    if (open) query = "";
  });

  function onSearch(): void {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      void form.find(query);
    }, 150);
  }

  async function submit(): Promise<void> {
    const saved = await form.submit();
    if (saved !== null) onSaved(saved);
  }
</script>

<Modal
  {open}
  title={form.id === null ? m.list_dialog_new() : m.list_dialog_edit()}
  width="560px"
  {onClose}
  locked={form.busy}
>
  <form
    class="list-form"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    <label class="field">
      <span>{m.list_name()}</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input"
        type="text"
        autocomplete="off"
        placeholder={m.list_name_placeholder()}
        bind:value={form.fields.name}
        autofocus
        disabled={form.busy}
      />
    </label>
    <label class="field">
      <span>{m.list_description()}</span>
      <input
        class="input"
        type="text"
        autocomplete="off"
        placeholder={m.list_description_hint()}
        bind:value={form.fields.description}
        disabled={form.busy}
      />
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={form.fields.bcc} disabled={form.busy} />
      <span>
        <span>{m.list_bcc()}</span>
        <span class="hint">{m.list_bcc_hint()}</span>
      </span>
    </label>

    <fieldset>
      <legend>{m.list_members_n({ count: form.members.length })}</legend>
      <label class="search">
        <SearchIcon size={16} aria-hidden="true" />
        <input
          class="input"
          type="search"
          autocomplete="off"
          placeholder={m.list_find_member_placeholder()}
          aria-label={m.list_find_member()}
          bind:value={query}
          oninput={onSearch}
          disabled={form.busy}
        />
      </label>
      {#if query.trim() !== ""}
        {#if form.candidates.length === 0}
          <p class="muted">{m.list_no_candidates()}</p>
        {:else}
          <ul class="candidates" aria-label={m.list_find_member()}>
            {#each form.candidates as c (c.id)}
              <li>
                <span class="who">
                  <span class="name">{c.name}</span>
                  {#if c.name !== c.address}<span class="address">{c.address}</span>{/if}
                </span>
                {#if form.isMember(c.id)}
                  <span class="muted member"
                    ><CheckIcon size={14} aria-hidden="true" />{m.list_already_member()}</span
                  >
                {:else}
                  <button
                    class="btn btn-ghost"
                    type="button"
                    aria-label={m.list_add_member({ name: c.name })}
                    disabled={form.busy || !form.canAdd}
                    onclick={() => form.add(c)}
                  >
                    <PlusIcon size={14} aria-hidden="true" />
                    <span>{m.list_add()}</span>
                  </button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
      {#if !form.canAdd}
        <p class="muted">{m.list_members_full()}</p>
      {/if}
      {#if form.members.length === 0}
        <p class="muted">{m.list_no_members_yet()}</p>
      {:else}
        <ul class="members">
          {#each form.members as member (member.id)}
            <li>
              <span class="who">
                <span class="name">{member.name}</span>
                {#if member.name !== member.address}<span class="address">{member.address}</span
                  >{/if}
              </span>
              <button
                class="btn btn-ghost icon"
                type="button"
                aria-label={m.list_remove_member({ name: member.name })}
                title={m.list_remove_member({ name: member.name })}
                disabled={form.busy}
                onclick={() => form.remove(member.id)}
              >
                <XIcon size={16} aria-hidden="true" />
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </fieldset>

    {#if form.error}
      <p class="error" role="alert">{form.error}</p>
    {/if}
    <div class="actions">
      <button class="btn" type="button" onclick={onClose} disabled={form.busy}
        >{m.common_cancel()}</button
      >
      <button class="btn btn-primary" type="submit" disabled={!form.canSubmit}>
        {form.busy ? m.list_saving() : m.list_save()}
      </button>
    </div>
  </form>
</Modal>

<style>
  .list-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .check {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    cursor: pointer;
  }

  .check > span {
    display: flex;
    flex-direction: column;
  }

  .check input {
    margin: var(--space-1) 0 0;
    accent-color: var(--color-accent);
  }

  .hint,
  .muted {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .muted {
    margin: 0;
  }

  fieldset {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: none;
  }

  legend {
    margin-bottom: var(--space-1);
    padding: 0;
  }

  .search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--color-text-muted);
  }

  .search .input {
    flex: 1;
    min-width: 0;
  }

  ul {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 12rem;
    overflow-y: auto;
  }

  .candidates {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
  }

  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-bottom: var(--border-width) solid var(--color-border);
  }

  li:last-child {
    border-bottom: none;
  }

  li:hover {
    background: var(--color-hover);
  }

  .who {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0;
  }

  .name {
    overflow-wrap: anywhere;
  }

  .address {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .member {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: var(--space-1);
  }

  .icon {
    flex: none;
    padding: var(--space-1);
    min-height: 0;
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
