<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Ajout ou modification d'un contact : noms, adresses (une préférée),
  // téléphones, organisation et fonction, anniversaire, adresses postales,
  // sites web, notes et favori. La fiche est revalidée par
  // Rust ; ses refus s'affichent au-dessus des boutons, sauf un nom qui
  // imite une adresse, expliqué sous son champ.
  import PlusIcon from "@lucide/svelte/icons/plus";
  import TrashIcon from "@lucide/svelte/icons/trash-2";
  import { tick } from "svelte";
  import Modal from "./Modal.svelte";
  import type { ContactDto } from "../lib/ipc/api";
  import type { ContactFormState, ContactNameField } from "../lib/contacts.svelte";
  import { monthNames } from "../lib/contact-details";
  import { activeLocale } from "../lib/i18n";

  interface Props {
    open: boolean;
    form: ContactFormState;
    onSaved: (contact: ContactDto) => void;
    onClose: () => void;
  }

  let { open, form, onSaved, onClose }: Props = $props();

  /** Mois proposés pour l'anniversaire, dans la langue de l'interface. */
  const months = $derived(monthNames(activeLocale()));

  /** Explication propre au champ `field`, s'il y en a une. */
  function fieldError(field: ContactNameField): string | null {
    return form.fieldError?.field === field ? form.fieldError.text : null;
  }

  async function submit(): Promise<void> {
    const saved = await form.submit();
    if (saved !== null) onSaved(saved);
    // Nom refusé : le champ reçoit le focus, son explication est lue avec
    // lui (et visible même si la fenêtre défilait plus bas).
    else if (form.fieldError !== null) {
      await tick();
      document.getElementById(`contact-${form.fieldError.field}`)?.focus();
    }
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
          id="contact-given_name"
          bind:value={form.fields.given_name}
          autofocus
          oninput={() => form.edited("given_name")}
          aria-invalid={fieldError("given_name") ? "true" : undefined}
          aria-describedby={fieldError("given_name") ? "contact-given_name-error" : undefined}
          disabled={form.busy}
        />
        {#if fieldError("given_name")}
          <span id="contact-given_name-error" class="field-error">{fieldError("given_name")}</span>
        {/if}
      </label>
      <label class="field">
        <span>{m.contact_family_name()}</span>
        <input
          class="input"
          type="text"
          autocomplete="off"
          id="contact-family_name"
          bind:value={form.fields.family_name}
          oninput={() => form.edited("family_name")}
          aria-invalid={fieldError("family_name") ? "true" : undefined}
          aria-describedby={fieldError("family_name") ? "contact-family_name-error" : undefined}
          disabled={form.busy}
        />
        {#if fieldError("family_name")}
          <span id="contact-family_name-error" class="field-error">{fieldError("family_name")}</span
          >
        {/if}
      </label>
      <label class="field">
        <span>{m.contact_display_name()}</span>
        <input
          class="input"
          type="text"
          autocomplete="off"
          placeholder={m.contact_display_name_hint()}
          id="contact-display_name"
          bind:value={form.fields.display_name}
          oninput={() => form.edited("display_name")}
          aria-invalid={fieldError("display_name") ? "true" : undefined}
          aria-describedby={fieldError("display_name") ? "contact-display_name-error" : undefined}
          disabled={form.busy}
        />
        {#if fieldError("display_name")}
          <span id="contact-display_name-error" class="field-error"
            >{fieldError("display_name")}</span
          >
        {/if}
      </label>
      <label class="field">
        <span>{m.contact_nickname()}</span>
        <input
          class="input"
          type="text"
          autocomplete="off"
          id="contact-nickname"
          bind:value={form.fields.nickname}
          oninput={() => form.edited("nickname")}
          aria-invalid={fieldError("nickname") ? "true" : undefined}
          aria-describedby={fieldError("nickname") ? "contact-nickname-error" : undefined}
          disabled={form.busy}
        />
        {#if fieldError("nickname")}
          <span id="contact-nickname-error" class="field-error">{fieldError("nickname")}</span>
        {/if}
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

    <div class="grid">
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
        <span>{m.contact_title()}</span>
        <input
          class="input"
          type="text"
          autocomplete="off"
          bind:value={form.fields.title}
          disabled={form.busy}
        />
      </label>
    </div>

    <fieldset>
      <legend>{m.contact_birthday()}</legend>
      <div class="row">
        <input
          class="input day"
          type="text"
          inputmode="numeric"
          maxlength="2"
          autocomplete="off"
          placeholder={m.contact_birthday_day()}
          aria-label={m.contact_birthday_day()}
          bind:value={form.birthday.day}
          disabled={form.busy}
        />
        <select
          class="input month"
          aria-label={m.contact_birthday_month()}
          bind:value={form.birthday.month}
          disabled={form.busy}
        >
          <option value="">{m.contact_birthday_no_month()}</option>
          {#each months as name, i (i)}
            <option value={String(i + 1)}>{name}</option>
          {/each}
        </select>
        <input
          class="input year"
          type="text"
          inputmode="numeric"
          maxlength="4"
          autocomplete="off"
          placeholder={m.contact_birthday_year()}
          aria-label={m.contact_birthday_year()}
          bind:value={form.birthday.year}
          disabled={form.busy}
        />
      </div>
    </fieldset>

    <fieldset>
      <legend>{m.contact_postal_addresses()}</legend>
      {#each form.fields.postal_addresses as place, i (i)}
        <div class="place">
          <textarea
            class="input street"
            rows="2"
            autocomplete="off"
            placeholder={m.contact_street_placeholder()}
            aria-label={m.contact_street_n({ n: i + 1 })}
            bind:value={place.street}
            disabled={form.busy}></textarea>
          <div class="row">
            <input
              class="input code"
              type="text"
              autocomplete="off"
              placeholder={m.contact_postal_code_placeholder()}
              aria-label={m.contact_postal_code_n({ n: i + 1 })}
              bind:value={place.postal_code}
              disabled={form.busy}
            />
            <input
              class="input address"
              type="text"
              autocomplete="off"
              placeholder={m.contact_locality_placeholder()}
              aria-label={m.contact_locality_n({ n: i + 1 })}
              bind:value={place.locality}
              disabled={form.busy}
            />
          </div>
          <div class="row">
            <input
              class="input address"
              type="text"
              autocomplete="off"
              placeholder={m.contact_region_placeholder()}
              aria-label={m.contact_region_n({ n: i + 1 })}
              bind:value={place.region}
              disabled={form.busy}
            />
            <input
              class="input address"
              type="text"
              autocomplete="off"
              placeholder={m.contact_country_placeholder()}
              aria-label={m.contact_country_n({ n: i + 1 })}
              bind:value={place.country}
              disabled={form.busy}
            />
          </div>
          <div class="row">
            <input
              class="input label"
              type="text"
              autocomplete="off"
              placeholder={m.contact_label_placeholder()}
              aria-label={m.contact_postal_label_n({ n: i + 1 })}
              bind:value={place.label}
              disabled={form.busy}
            />
            <button
              class="btn btn-ghost remove-place"
              type="button"
              title={m.contact_remove_postal()}
              aria-label={m.contact_remove_postal_n({ n: i + 1 })}
              disabled={form.busy}
              onclick={() => form.removePostalAddress(i)}
            >
              <TrashIcon size={16} aria-hidden="true" />
              <span>{m.contact_remove_postal()}</span>
            </button>
          </div>
        </div>
      {/each}
      {#if form.canAddPostalAddress}
        <button
          class="btn btn-ghost add"
          type="button"
          onclick={() => form.addPostalAddress()}
          disabled={form.busy}
        >
          <PlusIcon size={16} aria-hidden="true" />
          <span>{m.contact_add_postal()}</span>
        </button>
      {/if}
    </fieldset>

    <fieldset>
      <legend>{m.contact_websites()}</legend>
      {#each form.fields.websites as site, i (i)}
        <div class="row">
          <input
            class="input address"
            type="url"
            autocomplete="off"
            placeholder={m.contact_website_placeholder()}
            aria-label={m.contact_website_n({ n: i + 1 })}
            bind:value={site.url}
            disabled={form.busy}
          />
          <input
            class="input label"
            type="text"
            autocomplete="off"
            placeholder={m.contact_label_placeholder()}
            aria-label={m.contact_website_label_n({ n: i + 1 })}
            bind:value={site.label}
            disabled={form.busy}
          />
          <button
            class="btn btn-ghost icon"
            type="button"
            title={m.contact_remove_website()}
            aria-label={m.contact_remove_website_n({ n: i + 1 })}
            disabled={form.busy}
            onclick={() => form.removeWebsite(i)}
          >
            <TrashIcon size={16} aria-hidden="true" />
          </button>
        </div>
      {/each}
      {#if form.canAddWebsite}
        <button
          class="btn btn-ghost add"
          type="button"
          onclick={() => form.addWebsite()}
          disabled={form.busy}
        >
          <PlusIcon size={16} aria-hidden="true" />
          <span>{m.contact_add_website()}</span>
        </button>
      {/if}
    </fieldset>

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

  .row .day {
    flex: 0 0 5.5em;
    min-width: 0;
  }

  .row .month {
    flex: 2 1 0;
    min-width: 0;
  }

  .row .year {
    flex: 1 1 0;
    min-width: 0;
  }

  .row .code {
    flex: 0 0 8em;
    min-width: 0;
  }

  /* Une adresse postale : un bloc encadré, ses lignes empilées. */
  .place {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
  }

  .street {
    resize: vertical;
    font: inherit;
  }

  .remove-place {
    flex: none;
    gap: var(--space-1);
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

  .field-error {
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }
</style>
