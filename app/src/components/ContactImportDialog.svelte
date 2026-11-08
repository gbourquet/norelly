<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Aperçu d'un import de contacts : ce que l'import va faire (chiffres
  // décidés par Rust), sort des doublons, fiches rejetées et leur raison,
  // puis confirmation. Le texte du fichier (nom, repère d'une fiche) est
  // affiché échappé.
  import UserPlusIcon from "@lucide/svelte/icons/user-plus";
  import UserCheckIcon from "@lucide/svelte/icons/user-check";
  import CopyIcon from "@lucide/svelte/icons/copy";
  import TriangleAlertIcon from "@lucide/svelte/icons/triangle-alert";
  import Modal from "./Modal.svelte";
  import type { ContactImportState } from "../lib/contact-import.svelte";
  import type {
    ContactImportFormatDto,
    ContactImportOutcomeDto,
    ImportRejectReasonDto,
  } from "../lib/ipc/api";

  interface Props {
    open: boolean;
    form: ContactImportState;
    onImported: (outcome: ContactImportOutcomeDto) => void;
    onClose: () => void;
  }

  let { open, form, onImported, onClose }: Props = $props();

  function formatName(format: ContactImportFormatDto): string {
    switch (format) {
      case "vcard":
        return m.contacts_import_format_vcard();
      case "google_csv":
        return m.contacts_import_format_google_csv();
      case "outlook_csv":
        return m.contacts_import_format_outlook_csv();
      case "thunderbird_csv":
        return m.contacts_import_format_thunderbird_csv();
      case "ldif":
        return m.contacts_import_format_ldif();
    }
  }

  function reasonText(reason: ImportRejectReasonDto): string {
    switch (reason) {
      case "unreadable":
        return m.contacts_import_reason_unreadable();
      case "list":
        return m.contacts_import_reason_list();
      case "no_address":
        return m.contacts_import_reason_no_address();
      case "invalid_address":
        return m.contacts_import_reason_invalid_address();
      case "too_long":
        return m.contacts_import_reason_too_long();
      case "invalid_field":
        return m.contacts_import_reason_invalid_field();
      case "too_many_values":
        return m.contacts_import_reason_too_many_values();
    }
  }

  async function confirm(): Promise<void> {
    const outcome = await form.confirm();
    if (outcome !== null) onImported(outcome);
  }

  const preview = $derived(form.preview);
  const counts = $derived(form.counts);
</script>

<Modal {open} title={m.contacts_import_title()} width="560px" {onClose} locked={form.busy}>
  {#if preview !== null && counts !== null}
    <p class="file">
      {m.contacts_import_file({
        name: preview.file_name,
        format: formatName(preview.format),
        count: preview.cards,
      })}
    </p>
    <section aria-label={m.contacts_import_summary()}>
      <ul class="summary">
        <li>
          <UserPlusIcon size={16} aria-hidden="true" />
          <span>{m.contacts_import_create({ count: counts.create })}</span>
        </li>
        {#if counts.complete > 0}
          <li>
            <UserCheckIcon size={16} aria-hidden="true" />
            <span>{m.contacts_import_complete({ count: counts.complete })}</span>
          </li>
        {/if}
        {#if counts.duplicates > 0}
          <li>
            <CopyIcon size={16} aria-hidden="true" />
            <span>{m.contacts_import_duplicates({ count: counts.duplicates })}</span>
          </li>
        {/if}
        {#if preview.rejected_total > 0}
          <li class="warning">
            <TriangleAlertIcon size={16} aria-hidden="true" />
            <span>{m.contacts_import_rejected({ count: preview.rejected_total })}</span>
          </li>
        {/if}
      </ul>
    </section>
    {#if form.offersPolicy}
      <fieldset class="policy">
        <legend>{m.contacts_import_policy()}</legend>
        <label>
          <input type="radio" name="import-policy" value="skip" bind:group={form.policy} />
          <span>{m.contacts_import_policy_skip()}</span>
        </label>
        <label>
          <input type="radio" name="import-policy" value="complete" bind:group={form.policy} />
          <span>{m.contacts_import_policy_complete()}</span>
        </label>
      </fieldset>
    {/if}
    {#if preview.renamed > 0}
      <p class="note">{m.contacts_import_renamed({ count: preview.renamed })}</p>
    {/if}
    {#if preview.rejected.length > 0}
      <details class="rejected">
        <summary>{m.contacts_import_rejected_details()}</summary>
        <ul>
          {#each preview.rejected as r (r.card)}
            <li>
              {r.hint === ""
                ? m.contacts_import_rejection({ card: r.card, reason: reasonText(r.reason) })
                : m.contacts_import_rejection_named({
                    card: r.card,
                    hint: r.hint,
                    reason: reasonText(r.reason),
                  })}
            </li>
          {/each}
        </ul>
        {#if preview.rejected_total > preview.rejected.length}
          <p>
            {m.contacts_import_rejected_more({
              count: preview.rejected_total - preview.rejected.length,
            })}
          </p>
        {/if}
      </details>
    {/if}
    {#if counts.create + counts.complete === 0}
      <p class="note" role="status">{m.contacts_import_nothing()}</p>
    {/if}
    {#if form.error}
      <p class="error" role="alert">{form.error}</p>
    {/if}
  {/if}
  <div class="actions">
    <button class="btn" type="button" onclick={onClose} disabled={form.busy}
      >{m.common_cancel()}</button
    >
    <button
      class="btn btn-primary"
      type="button"
      onclick={() => void confirm()}
      disabled={!form.canImport}
    >
      {form.busy ? m.contacts_import_busy() : m.contacts_import_confirm()}
    </button>
  </div>
</Modal>

<style>
  .file {
    margin: 0 0 var(--space-3);
    overflow-wrap: anywhere;
  }

  .summary {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .summary li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .summary .warning {
    color: var(--color-warning);
  }

  .policy {
    margin: var(--space-4) 0 0;
    padding: var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .policy legend {
    padding: 0 var(--space-1);
    font-weight: var(--font-weight-medium);
  }

  .policy label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
  }

  .note {
    color: var(--color-text-muted);
    margin: var(--space-3) 0 0;
  }

  .rejected {
    margin-top: var(--space-3);
  }

  .rejected summary {
    cursor: pointer;
    color: var(--color-accent);
  }

  .rejected summary:focus-visible {
    outline: 2px solid var(--color-focus);
    outline-offset: 2px;
  }

  .rejected ul {
    max-height: 160px;
    overflow-y: auto;
    margin: var(--space-2) 0 0;
    padding-left: var(--space-5);
    overflow-wrap: anywhere;
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
