<script lang="ts">
  // Réglages des étiquettes d'un compte : ajouter, colorer (couleur
  // locale), renommer, supprimer (de la liste, ou aussi de tous les
  // messages). Chaque changement est immédiat ; Rust revalide tout.
  import { m } from "../lib/paraglide/messages.js";
  import type { AccountDto } from "../lib/ipc/api";
  import { labelText, type LabelSettingsState, type LabelsState } from "../lib/labels.svelte";
  import LabelChip from "./LabelChip.svelte";

  interface Props {
    accounts: AccountDto[];
    state: LabelSettingsState;
    labels: LabelsState;
  }

  let { accounts, state, labels }: Props = $props();

  const list = $derived(state.account === null ? [] : labels.of(state.account));

  // Compte choisi par défaut : le premier ; liste relue à chaque choix.
  $effect(() => {
    if (state.account === null || !accounts.some((a) => a.id === state.account)) {
      state.account = accounts[0]?.id ?? null;
    }
  });
  $effect(() => {
    if (state.account !== null) void labels.load(state.account);
  });

  /** Valeur du sélecteur de couleur (gris neutre sans couleur). */
  const NEUTRAL = "#9aa3ad";
</script>

<section class="labels-settings" aria-labelledby="labels-settings-title">
  <h2 id="labels-settings-title">{m.settings_labels()}</h2>
  {#if accounts.length > 1}
    <label class="row">
      <span>{m.settings_labels_account()}</span>
      <select bind:value={state.account}>
        {#each accounts as account (account.id)}
          <option value={account.id}>{account.address}</option>
        {/each}
      </select>
    </label>
  {/if}
  {#if list.length === 0}
    <p class="note">{m.settings_labels_empty()}</p>
  {:else}
    <ul class="list">
      {#each list as label (label.keyword)}
        {@const text = labelText(label)}
        <li class="item">
          {#if state.renaming?.keyword === label.keyword}
            <input
              class="grow"
              type="text"
              aria-label={m.settings_label_rename_field({ label: text })}
              bind:value={state.renaming.name}
              maxlength="40"
              onkeydown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void state.rename();
                }
              }}
            />
            <button
              class="btn"
              type="button"
              disabled={state.busy}
              onclick={() => void state.rename()}>{m.settings_label_rename_confirm()}</button
            >
            <button class="btn" type="button" onclick={() => (state.renaming = null)}
              >{m.common_cancel()}</button
            >
          {:else if state.deleting?.keyword === label.keyword}
            <div class="grow confirm">
              <span>{m.settings_label_delete_confirm({ label: text })}</span>
              <label class="check">
                <input type="checkbox" bind:checked={state.deleting.everywhere} />
                {m.settings_label_delete_everywhere()}
              </label>
            </div>
            <button
              class="btn btn-danger"
              type="button"
              disabled={state.busy}
              onclick={() => void state.remove()}>{m.settings_label_delete_confirm_button()}</button
            >
            <button class="btn" type="button" onclick={() => (state.deleting = null)}
              >{m.common_cancel()}</button
            >
          {:else}
            <span class="grow chip"><LabelChip {label} /></span>
            <input
              class="color"
              type="color"
              aria-label={m.settings_label_color({ label: text })}
              title={m.settings_label_color({ label: text })}
              value={label.color?.background ?? NEUTRAL}
              disabled={state.busy}
              onchange={(e) => void state.setColor(label, e.currentTarget.value)}
            />
            {#if label.color !== null}
              <button
                class="btn"
                type="button"
                disabled={state.busy}
                onclick={() => void state.setColor(label, null)}
                >{m.settings_label_no_color()}</button
              >
            {/if}
            <button class="btn" type="button" onclick={() => state.startRename(label)}
              >{m.settings_label_rename()}</button
            >
            <button class="btn" type="button" onclick={() => state.startDelete(label)}
              >{m.settings_label_delete()}</button
            >
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  <div class="item">
    <input
      class="grow"
      type="text"
      aria-label={m.settings_labels_new()}
      placeholder={m.settings_labels_new()}
      bind:value={state.newName}
      maxlength="40"
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          void state.create();
        }
      }}
    />
    <button
      class="btn"
      type="button"
      disabled={state.busy || state.newName.trim() === "" || state.account === null}
      onclick={() => void state.create()}>{m.settings_labels_add()}</button
    >
  </div>
  {#if state.notice}
    <p class="note" role="status">{state.notice}</p>
  {/if}
  {#if state.error}
    <p class="error" role="alert">{state.error}</p>
  {/if}
  <p class="note">{m.settings_labels_note()}</p>
</section>

<style>
  .labels-settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-3);
    border-top: var(--border-width) solid var(--color-border);
  }

  h2 {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-bold);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 220px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .chip {
    display: flex;
    overflow: hidden;
  }

  .confirm {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
  }

  .color {
    flex: none;
    width: 32px;
    height: 28px;
    padding: 0;
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: none;
    cursor: pointer;
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
</style>
