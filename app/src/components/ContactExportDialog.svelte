<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Export de contacts : ce qui sera exporté, choix du format, puis
  // sélecteur d'enregistrement du système. Le fichier est produit et écrit
  // par Rust.
  import Modal from "./Modal.svelte";
  import type { ContactExportState } from "../lib/contact-export.svelte";
  import type { ContactExportFormatDto, ContactExportOutcomeDto } from "../lib/ipc/api";

  interface Props {
    open: boolean;
    form: ContactExportState;
    onExported: (outcome: ContactExportOutcomeDto) => void;
    onClose: () => void;
  }

  let { open, form, onExported, onClose }: Props = $props();

  const formats: { value: ContactExportFormatDto; name: () => string; hint: () => string }[] = [
    {
      value: "vcard4",
      name: m.contacts_export_format_vcard4,
      hint: m.contacts_export_format_vcard4_hint,
    },
    {
      value: "vcard3",
      name: m.contacts_export_format_vcard3,
      hint: m.contacts_export_format_vcard3_hint,
    },
    {
      value: "google_csv",
      name: m.contacts_export_format_google_csv,
      hint: m.contacts_export_format_google_csv_hint,
    },
    {
      value: "ldif",
      name: m.contacts_export_format_ldif,
      hint: m.contacts_export_format_ldif_hint,
    },
  ];

  function filterName(format: ContactExportFormatDto): string {
    switch (format) {
      case "vcard4":
      case "vcard3":
        return m.contacts_export_filter_vcard();
      case "google_csv":
        return m.contacts_export_filter_csv();
      case "ldif":
        return m.contacts_export_filter_ldif();
    }
  }

  async function confirm(): Promise<void> {
    const outcome = await form.confirm((format) => ({
      title: m.contacts_export_pick_title(),
      filter: filterName(format),
    }));
    if (outcome !== null) onExported(outcome);
  }
</script>

<Modal {open} title={m.contacts_export_title()} width="520px" {onClose} locked={form.busy}>
  <p class="scope">
    {form.count === null
      ? m.contacts_export_scope_all()
      : m.contacts_export_scope_selected({ count: form.count })}
  </p>
  <fieldset class="formats">
    <legend>{m.contacts_export_format()}</legend>
    {#each formats as format (format.value)}
      <label>
        <input
          type="radio"
          name="export-format"
          value={format.value}
          bind:group={form.format}
          disabled={form.busy}
        />
        <span class="text">
          <span class="name">{format.name()}</span>
          <span class="hint">{format.hint()}</span>
        </span>
      </label>
    {/each}
  </fieldset>
  <p class="note">{m.contacts_export_note()}</p>
  {#if form.error}
    <p class="error" role="alert">{form.error}</p>
  {/if}
  <div class="actions">
    <button class="btn" type="button" onclick={onClose} disabled={form.busy}
      >{m.common_cancel()}</button
    >
    <button
      class="btn btn-primary"
      type="button"
      onclick={() => void confirm()}
      disabled={form.busy || form.selection === null}
    >
      {form.busy ? m.contacts_export_busy() : m.contacts_export_confirm()}
    </button>
  </div>
</Modal>

<style>
  .scope {
    margin: 0 0 var(--space-3);
  }

  .formats {
    margin: 0;
    padding: var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .formats legend {
    padding: 0 var(--space-1);
    font-weight: var(--font-weight-medium);
  }

  .formats label {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    cursor: pointer;
  }

  .formats input {
    margin-top: 3px;
  }

  .text {
    display: flex;
    flex-direction: column;
  }

  .hint,
  .note {
    color: var(--color-text-muted);
  }

  .note {
    margin: var(--space-3) 0 0;
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
