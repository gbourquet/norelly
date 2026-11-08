<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Doublons du carnet : un groupe à la fois. Fiches du groupe (nom,
  // adresses, téléphones), case « réunir », choix de la fiche gardée,
  // puis « Fusionner » ou « Passer ». La fusion est faite par Rust.
  import Modal from "./Modal.svelte";
  import type { ContactDuplicatesState } from "../lib/contact-duplicates.svelte";
  import type { ContactDto, DuplicateReasonDto } from "../lib/ipc/api";

  interface Props {
    open: boolean;
    form: ContactDuplicatesState;
    onMerged: (kept: ContactDto) => void;
    onClose: () => void;
  }

  let { open, form, onMerged, onClose }: Props = $props();

  function reason(r: DuplicateReasonDto): string {
    switch (r) {
      case "same_name":
        return m.contacts_duplicates_reason_same_name();
      case "same_phone":
        return m.contacts_duplicates_reason_same_phone();
    }
  }

  async function merge(): Promise<void> {
    const kept = await form.merge();
    if (kept !== null) onMerged(kept);
  }
</script>

<Modal {open} title={m.contacts_duplicates_title()} width="640px" {onClose} locked={form.busy}>
  {#if form.loading}
    <p class="status" role="status">{m.contacts_duplicates_searching()}</p>
  {:else if form.current}
    {@const group = form.current}
    <div class="heading">
      <p class="progress">
        {m.contacts_duplicates_progress({ position: form.position, total: form.total })}
      </p>
      <ul class="reasons" aria-label={m.contacts_duplicates_reasons()}>
        {#each group.reasons as r (r)}
          <li class="reason">{reason(r)}</li>
        {/each}
      </ul>
    </div>
    <p class="help">{m.contacts_duplicates_help()}</p>
    <ul class="cards">
      {#each group.contacts as contact (contact.id)}
        {@const kept = form.keep === contact.id}
        {@const included = form.isChecked(contact.id)}
        <li class="card" class:kept class:excluded={!included}>
          <label class="include">
            <input
              type="checkbox"
              checked={included}
              disabled={kept || form.busy}
              aria-label={m.contacts_duplicates_include({ name: contact.name })}
              onchange={() => form.toggle(contact.id)}
            />
            <span class="name">{contact.name}</span>
          </label>
          <div class="details">
            {#each contact.fields.emails as email (email.address)}
              <span class="line">{email.address}</span>
            {/each}
            {#each contact.fields.phones as phone, i (i)}
              <span class="line muted">{phone.number}</span>
            {/each}
            {#if contact.fields.organization}
              <span class="line muted">{contact.fields.organization}</span>
            {/if}
          </div>
          <label class="keep">
            <input
              type="radio"
              name="duplicate-keep"
              checked={kept}
              disabled={form.busy}
              onchange={() => form.choose(contact.id)}
            />
            {m.contacts_duplicates_keep()}
          </label>
        </li>
      {/each}
    </ul>
  {:else if form.error === null}
    <p class="status" role="status">
      {form.merged > 0 || form.total > 0
        ? m.contacts_duplicates_done()
        : m.contacts_duplicates_none()}
    </p>
  {/if}
  {#if form.error}
    <p class="error" role="alert">{form.error}</p>
  {/if}
  <div class="actions">
    <button class="btn" type="button" onclick={onClose} disabled={form.busy}
      >{m.common_close()}</button
    >
    {#if form.current}
      <button class="btn" type="button" onclick={() => form.skip()} disabled={form.busy}
        >{m.contacts_duplicates_skip()}</button
      >
      <button
        class="btn btn-primary"
        type="button"
        onclick={() => void merge()}
        disabled={!form.canMerge}
      >
        {form.busy ? m.contacts_duplicates_merging() : m.contacts_duplicates_merge()}
      </button>
    {/if}
  </div>
</Modal>

<style>
  .status,
  .help {
    margin: 0 0 var(--space-3);
  }

  .help,
  .muted {
    color: var(--color-text-muted);
  }

  .heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .progress {
    margin: 0;
    font-weight: var(--font-weight-medium);
  }

  .reasons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .reason {
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
    color: var(--color-text-muted);
  }

  .cards {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 50vh;
    overflow-y: auto;
  }

  .card {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--space-1) var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
  }

  .card.kept {
    border-color: var(--color-accent);
  }

  .card.excluded {
    opacity: 0.6;
  }

  .include {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    cursor: pointer;
  }

  .name {
    font-weight: var(--font-weight-medium);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .details {
    grid-column: 1;
    display: flex;
    flex-direction: column;
    padding-left: calc(var(--space-2) + 16px);
    min-width: 0;
  }

  .line {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .keep {
    grid-column: 2;
    grid-row: 1;
    display: flex;
    align-items: center;
    gap: var(--space-1);
    white-space: nowrap;
    cursor: pointer;
  }

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
