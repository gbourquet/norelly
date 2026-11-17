<script lang="ts">
  // « Étiqueter… » : étiquettes du compte, cochées si tous les messages
  // visés les portent (décision de Rust) ; cocher pose, décocher retire,
  // tout de suite. Une étiquette refusée par le serveur est désactivée,
  // avec une explication. Une nouvelle étiquette se crée et se pose d'un
  // coup.
  import { m } from "../lib/paraglide/messages.js";
  import Modal from "./Modal.svelte";
  import LabelChip from "./LabelChip.svelte";
  import type { LabelMenuState } from "../lib/labels.svelte";

  interface Props {
    open: boolean;
    state: LabelMenuState;
    onClose: () => void;
  }

  let { open, state, onClose }: Props = $props();

  const menu = $derived(state.menu);
</script>

<Modal {open} title={m.label_dialog_title()} description={m.label_dialog_description()} {onClose}>
  <div class="labels-dialog">
    {#if menu === null}
      <p class="note">{m.label_dialog_loading()}</p>
    {:else}
      {#if menu.choices.length === 0}
        <p class="note">{m.label_dialog_empty()}</p>
      {:else}
        <ul class="choices">
          {#each menu.choices as choice (choice.label.keyword)}
            <li>
              <label
                class="choice"
                class:refused={!choice.allowed}
                title={choice.allowed ? undefined : m.label_choice_refused()}
              >
                <input
                  type="checkbox"
                  checked={choice.checked}
                  disabled={!choice.allowed || state.busy}
                  onchange={() => void state.toggle(choice)}
                />
                <LabelChip label={choice.label} />
              </label>
            </li>
          {/each}
        </ul>
      {/if}
      <form
        class="create"
        onsubmit={(e) => {
          e.preventDefault();
          void state.create();
        }}
      >
        <input
          type="text"
          aria-label={m.label_dialog_new()}
          placeholder={m.label_dialog_new()}
          bind:value={state.newName}
          disabled={!menu.can_create || state.busy}
          title={menu.can_create ? undefined : m.label_dialog_cannot_create()}
          maxlength="40"
        />
        <button
          class="btn"
          type="submit"
          disabled={!menu.can_create || state.busy || state.newName.trim() === ""}
          title={menu.can_create ? undefined : m.label_dialog_cannot_create()}
          >{m.label_dialog_create()}</button
        >
      </form>
      {#if !menu.can_create}
        <p class="note">{m.label_dialog_cannot_create()}</p>
      {/if}
    {/if}
    {#if state.error}
      <p class="error" role="alert">{state.error}</p>
    {/if}
    <div class="actions">
      <button class="btn btn-primary" type="button" onclick={onClose}
        >{m.label_dialog_done()}</button
      >
    </div>
  </div>
</Modal>

<style>
  .labels-dialog {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .choices {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 320px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .choice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius);
    cursor: pointer;
  }

  .choice:hover {
    background: var(--color-hover);
  }

  .choice.refused {
    color: var(--color-text-muted);
    cursor: not-allowed;
  }

  .choice input {
    margin: 0;
    accent-color: var(--color-accent);
  }

  .create {
    display: flex;
    gap: var(--space-2);
  }

  .create input {
    flex: 1;
    min-width: 0;
  }

  .note {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .error {
    margin: 0;
    color: var(--color-danger);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }
</style>
