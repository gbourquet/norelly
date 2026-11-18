<script lang="ts" generics="C">
  import { m } from "../lib/paraglide/messages.js";
  // Fiche d'un contact, en lecture : barre d'actions (écrire, modifier,
  // supprimer, favori), adresses, téléphones, organisation et fonction,
  // anniversaire, adresses postales, sites web et notes. Tout est affiché
  // en texte ; un site s'ouvre par Rust (open_external, qui le revalide),
  // son adresse entière visible sur le bouton.
  import StarIcon from "@lucide/svelte/icons/star";
  import MailIcon from "@lucide/svelte/icons/mail";
  import PhoneIcon from "@lucide/svelte/icons/phone";
  import BuildingIcon from "@lucide/svelte/icons/building-2";
  import CakeIcon from "@lucide/svelte/icons/cake";
  import MapPinIcon from "@lucide/svelte/icons/map-pin";
  import GlobeIcon from "@lucide/svelte/icons/globe";
  import ExternalLinkIcon from "@lucide/svelte/icons/external-link";
  import { birthdayText, postalLines } from "../lib/contact-details";
  import { activeLocale } from "../lib/i18n";
  import { errorText } from "../lib/errors";
  import type { ContactDto } from "../lib/ipc/api";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { contactsToolbar } from "../lib/visible-actions";
  import { contactLabel } from "../lib/contact-label";
  import ActionBar from "./ActionBar.svelte";

  interface Props {
    contact: ContactDto;
    registry: CommandRegistry<C>;
    context: C;
    /** Ouvre un site de la fiche dans le navigateur du système. */
    openWebsite: (url: string) => Promise<void>;
  }

  let { contact, registry, context, openWebsite }: Props = $props();

  /** Échec de l'ouverture d'un site, affiché sous la liste des sites. */
  let websiteError = $state<string | null>(null);

  $effect(() => {
    void contact.id;
    websiteError = null;
  });

  async function open(url: string): Promise<void> {
    websiteError = null;
    try {
      await openWebsite(url);
    } catch (e) {
      websiteError = m.contacts_website_failed({ error: errorText(e) });
    }
  }

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
            {#if email.label !== ""}<span class="tag">{contactLabel(email)}</span>{/if}
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
              {#if phone.label !== ""}<span class="tag">{contactLabel(phone)}</span>{/if}
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if f.organization !== "" || f.title !== ""}
      <section aria-label={m.contacts_organization()}>
        <h3><BuildingIcon size={16} aria-hidden="true" />{m.contacts_organization()}</h3>
        {#if f.title !== ""}<p class="value">{f.title}</p>{/if}
        {#if f.organization !== ""}<p class="value">{f.organization}</p>{/if}
      </section>
    {/if}

    {#if f.birthday !== ""}
      <section aria-label={m.contacts_birthday()}>
        <h3><CakeIcon size={16} aria-hidden="true" />{m.contacts_birthday()}</h3>
        <p class="value">{birthdayText(f.birthday, activeLocale())}</p>
      </section>
    {/if}

    {#if f.postal_addresses.length > 0}
      <section aria-label={m.contacts_postal_addresses()}>
        <h3><MapPinIcon size={16} aria-hidden="true" />{m.contacts_postal_addresses()}</h3>
        <ul>
          {#each f.postal_addresses as place, i (i)}
            <li class="place">
              <address class="value">
                {#each postalLines(place) as line, j (j)}<span class="line">{line}</span>{/each}
              </address>
              {#if place.label !== ""}<span class="tag">{contactLabel(place)}</span>{/if}
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if f.websites.length > 0}
      <section aria-label={m.contacts_websites()}>
        <h3><GlobeIcon size={16} aria-hidden="true" />{m.contacts_websites()}</h3>
        <ul>
          {#each f.websites as site, i (i)}
            <li>
              <button
                class="link"
                type="button"
                title={m.contacts_open_website({ url: site.url })}
                onclick={() => open(site.url)}
              >
                <span class="value">{site.url}</span>
                <ExternalLinkIcon size={14} aria-hidden="true" />
              </button>
              {#if site.label !== ""}<span class="tag">{contactLabel(site)}</span>{/if}
            </li>
          {/each}
        </ul>
        {#if websiteError !== null}<p class="error" role="alert">{websiteError}</p>{/if}
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

  .place {
    align-items: flex-start;
  }

  address {
    display: flex;
    flex-direction: column;
    font-style: normal;
  }

  .link {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 0;
    border: none;
    background: none;
    color: var(--color-accent);
    font: inherit;
    text-align: left;
    text-decoration: underline;
    cursor: pointer;
  }

  .link:hover {
    text-decoration-thickness: 2px;
  }

  .error {
    margin: var(--space-2) 0 0;
    color: var(--color-danger);
  }

  .notes {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
</style>
