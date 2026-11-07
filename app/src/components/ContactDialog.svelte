<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Ajout ou modification d'un contact : noms, adresses (une préférée),
  // téléphones, organisation, notes et favori. La fiche est revalidée par
  // Rust ; ses refus s'affichent au-dessus des boutons.
  import PlusIcon from "@lucide/svelte/icons/plus";
  import TrashIcon from "@lucide/svelte/icons/trash-2";
  import Modal from "./Modal.svelte";
  import type { ContactDto } from "../lib/ipc/api";
  import type { ContactFormState } from "../lib/contacts.svelte";

  interface Props {
    open: boolean;
    form: ContactFormState;
    onSaved: (contact: ContactDto) => void;
    onClose: () => void;
  }

  let { open, form, onSaved, onClose }: Props = $props();

  async function submit(): Promise<void> {
    const saved = await form.submit();
    if (saved !== null) onSaved(saved);
  }
</script>

<Modal
  {open}
  title={form.id === null ? m.contact_dialog_new() : m.contact_dialog_edit()}
  width="560px"
  {onClose}
  locked={form.busy}
>
  <form
    class="contact-form"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    <div class="grid">
      <label class="field">
        <span>{m.contact_given_name()}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="input"
          type="text"
          autocomplete="off"
          bind:value={form.fields.given_name}
          autofocus
          disabled={form.busy}
        />
      </label>
      <label class="field">
        <span>{m.contact_family_name()}</span>
        <input
          class="input"
          type="text"
          autocomplete="off"
          bind:value={form.fields.family_name}
          disabled={form.busy}
        />
      </label>
      <label class="field">
        <span>{m.contact_display_name()}</span>
        <input
          class="input"
          type="text"
          autocomplete="off"
          placeholder={m.contact_display_name_hint()}
          bind:value={form.fields.display_name}
          disabled={form.busy}
        />
      </label>
      <label class="field">
        <span>{m.contact_nickname()}</span>
        <input
          class="input"
          type="text"
          autocomplete="off"
          bind:value={form.fields.nickname}
          disabled={form.busy}
        />
      </label>
    </div>

    <fieldset>
      <legend>{m.contact_emails()}</legend>
      {#each form.fields.emails as email, i (i)}
        <div class="row">
          <input
            class="input address"
            type="email"
            autocomplete="off"
            placeholder={m.contact_email_placeholder()}
            aria-label={m.contact_email_n({ n: i + 1 })}
            bind:value={email.address}
            disabled={form.busy}
          />
          <input
            class="input label"
            type="text"
            autocomplete="off"
            placeholder={m.contact_label_placeholder()}
            aria-label={m.contact_email_label_n({ n: i + 1 })}
            bind:value={email.label}
            disabled={form.busy}
          />
          <label class="preferred" title={m.contact_preferred_hint()}>
            <input
              type="radio"
              name="preferred"
              checked={email.preferred}
              onchange={() => form.setPreferred(i)}
              disabled={form.busy}
            />
            <span>{m.contact_preferred()}</span>
          </label>
          <button
            class="btn btn-ghost icon"
            type="button"
            title={m.contact_remove_email()}
            aria-label={m.contact_remove_email_n({ n: i + 1 })}
            disabled={form.busy || form.fields.emails.length <= 1}
            onclick={() => form.removeEmail(i)}
          >
            <TrashIcon size={16} aria-hidden="true" />
          </button>
        </div>
      {/each}
      {#if form.canAddEmail}
        <button
          class="btn btn-ghost add"
          type="button"
          onclick={() => form.addEmail()}
          disabled={form.busy}
        >
          <PlusIcon size={16} aria-hidden="true" />
          <span>{m.contact_add_email()}</span>
        </button>
      {/if}
    </fieldset>

    <fieldset>
      <legend>{m.contact_phones()}</legend>
      {#each form.fields.phones as phone, i (i)}
        <div class="row">
          <input
            class="input address"
            type="tel"
            autocomplete="off"
            aria-label={m.contact_phone_n({ n: i + 1 })}
            bind:value={phone.number}
            disabled={form.busy}
          />
          <input
            class="input label"
            type="text"
            autocomplete="off"
            placeholder={m.contact_label_placeholder()}
            aria-label={m.contact_phone_label_n({ n: i + 1 })}
            bind:value={phone.label}
            disabled={form.busy}
          />
          <button
            class="btn btn-ghost icon"
            type="button"
            title={m.contact_remove_phone()}
            aria-label={m.contact_remove_phone_n({ n: i + 1 })}
            disabled={form.busy}
            onclick={() => form.removePhone(i)}
          >
            <TrashIcon size={16} aria-hidden="true" />
          </button>
        </div>
      {/each}
      {#if form.canAddPhone}
        <button
          class="btn btn-ghost add"
          type="button"
          onclick={() => form.addPhone()}
          disabled={form.busy}
        >
          <PlusIcon size={16} aria-hidden="true" />
          <span>{m.contact_add_phone()}</span>
        </button>
      {/if}
    </fieldset>

    <label class="field">
      <span>{m.contact_organization()}</span>
      <input
        class="input"
        type="text"
        autocomplete="off"
        bind:value={form.fields.organization}
        disabled={form.busy}
      />
    </label>
    <label class="field">
      <span>{m.contact_notes()}</span>
      <textarea class="input notes" rows="2" bind:value={form.fields.notes} disabled={form.busy}
      ></textarea>
    </label>

    {#if form.error}
      <p class="error" role="alert">{form.error}</p>
    {/if}
    <div class="actions">
      <label class="check">
        <input type="checkbox" bind:checked={form.fields.favorite} disabled={form.busy} />
        <span>{m.contact_favorite()}</span>
      </label>
      <button class="btn" type="button" onclick={onClose} disabled={form.busy}
        >{m.common_cancel()}</button
      >
      <button class="btn btn-primary" type="submit" disabled={!form.canSubmit}>
        {form.busy ? m.contact_saving() : m.contact_save()}
      </button>
    </div>
  </form>
</Modal>

<style>
  .contact-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: var(--space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
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

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .row .address {
    flex: 2 1 0;
    min-width: 0;
  }

  .row .label {
    flex: 1 1 0;
    min-width: 0;
  }

  .preferred,
  .check {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--font-size-sm);
    cursor: pointer;
  }

  .preferred input,
  .check input {
    margin: 0;
    accent-color: var(--color-accent);
  }

  .icon {
    flex: none;
    padding: var(--space-1);
    min-height: 0;
  }

  .add {
    align-self: flex-start;
  }

  .notes {
    resize: vertical;
    font: inherit;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  /* Favori à gauche, boutons à droite. */
  .actions .check {
    margin-right: auto;
  }

  .error {
    margin: 0;
    color: var(--color-danger);
  }
</style>
