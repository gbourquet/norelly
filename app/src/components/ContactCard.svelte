<script lang="ts" generics="C">
  import { m } from "../lib/paraglide/messages.js";
  // Fiche d'un contact, en lecture : barre d'actions (écrire, modifier,
  // supprimer, favori), adresses, téléphones, organisation et notes. Tout
  // est affiché en texte.
  import StarIcon from "@lucide/svelte/icons/star";
  import MailIcon from "@lucide/svelte/icons/mail";
  import PhoneIcon from "@lucide/svelte/icons/phone";
  import BuildingIcon from "@lucide/svelte/icons/building-2";
  import type { ContactDto } from "../lib/ipc/api";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { contactsToolbar } from "../lib/visible-actions";
  import ActionBar from "./ActionBar.svelte";

  interface Props {
    contact: ContactDto;
    registry: CommandRegistry<C>;
    context: C;
  }

  let { contact, registry, context }: Props = $props();

  const actions = $derived(contactsToolbar({ favorite: contact.fields.favorite }));
  const f = $derived(contact.fields);
  /** Prénom et nom, s'ils diffèrent du nom affiché. */
  const fullName = $derived([f.given_name, f.family_name].filter((x) => x !== "").join(" "));
</script>

<article class="card" aria-label={m.contacts_card({ name: contact.name })}>
  <div class="toolbar">
    <ActionBar
      primary={actions.primary}
      more={actions.more}
      {registry}
      {context}
      label={m.contacts_card_toolbar()}
    />
  </div>
  <div class="body">
    <header>
      <div class="avatar" aria-hidden="true">{contact.name.trim().charAt(0).toUpperCase()}</div>
      <div class="titles">
        <h2>
          {contact.name}
          {#if f.favorite}<span class="star"
              ><StarIcon size={18} aria-label={m.contacts_is_favorite()} /></span
            >{/if}
        </h2>
        {#if fullName !== "" && fullName !== contact.name}<p class="sub">{fullName}</p>{/if}
        {#if f.nickname !== ""}<p class="sub">
            {m.contacts_nickname_shown({ nickname: f.nickname })}
          </p>{/if}
      </div>
    </header>

    <section aria-label={m.contacts_emails()}>
      <h3><MailIcon size={16} aria-hidden="true" />{m.contacts_emails()}</h3>
      <ul>
        {#each f.emails as email (email.address)}
          <li>
            <span class="value">{email.address}</span>
            {#if email.label !== ""}<span class="tag">{email.label}</span>{/if}
            {#if email.preferred && f.emails.length > 1}<span class="tag preferred"
                >{m.contacts_preferred()}</span
              >{/if}
          </li>
        {/each}
      </ul>
    </section>

    {#if f.phones.length > 0}
      <section aria-label={m.contacts_phones()}>
        <h3><PhoneIcon size={16} aria-hidden="true" />{m.contacts_phones()}</h3>
        <ul>
          {#each f.phones as phone, i (i)}
            <li>
              <span class="value">{phone.number}</span>
              {#if phone.label !== ""}<span class="tag">{phone.label}</span>{/if}
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if f.organization !== ""}
      <section aria-label={m.contacts_organization()}>
        <h3><BuildingIcon size={16} aria-hidden="true" />{m.contacts_organization()}</h3>
        <p class="value">{f.organization}</p>
      </section>
    {/if}

    {#if f.notes !== ""}
      <section aria-label={m.contacts_notes()}>
        <h3>{m.contacts_notes()}</h3>
        <p class="notes">{f.notes}</p>
      </section>
    {/if}
  </div>
</article>

<style>
  .card {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .toolbar {
    padding: var(--space-2) var(--space-4);
    border-bottom: var(--border-width) solid var(--color-border);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-6);
    overflow-y: auto;
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .avatar {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: var(--color-selected);
    color: var(--color-accent);
    font-size: var(--font-size-xl);
    font-weight: var(--font-weight-bold);
  }

  .titles {
    min-width: 0;
  }

  h2 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--font-size-xl);
    font-weight: var(--font-weight-medium);
    overflow-wrap: anywhere;
  }

  .star {
    display: inline-flex;
    color: var(--color-warning);
  }

  .sub {
    margin: var(--space-1) 0 0;
    color: var(--color-text-muted);
  }

  h3 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .value {
    overflow-wrap: anywhere;
    user-select: text;
  }

  p.value {
    margin: 0;
  }

  .tag {
    padding: 0 var(--space-2);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .tag.preferred {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }

  .notes {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
</style>
