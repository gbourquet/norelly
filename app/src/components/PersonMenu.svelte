<script lang="ts" generics="C">
  import { m } from "../lib/paraglide/messages.js";
  // Nom d'une personne d'un message (expéditeur, destinataire) : un clic
  // ouvre son menu (« Ajouter au carnet d'adresses » ou « Voir le
  // contact ») ; une pastille signale une personne du carnet. Le nom
  // affiché est choisi par Rust (carnet, sinon en-tête).
  import BookUserIcon from "@lucide/svelte/icons/book-user";
  import type { CorrespondentDto } from "../lib/ipc/api";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { personActions, type PersonKind } from "../lib/visible-actions";
  import { personName } from "../lib/format";
  import CommandMenu from "./CommandMenu.svelte";

  interface Props {
    person: CorrespondentDto;
    registry: CommandRegistry<C>;
    /** Contexte des commandes qui visent cette personne. */
    context: C;
    /** Exécute une entrée du menu pour cette personne. */
    run: (id: string) => void;
    /** Nom en gras (message non lu). */
    strong?: boolean;
  }

  let { person, registry, context, run, strong = false }: Props = $props();

  const kind = $derived<PersonKind>(person.contact !== null ? "contact" : "stranger");
  const name = $derived(personName(person));
  const label = $derived(
    person.contact !== null
      ? `${m.person_menu({ name })} (${m.person_in_contacts()})`
      : m.person_menu({ name }),
  );
</script>

<CommandMenu actions={personActions(kind)} {registry} {context} {label} {run} align="start">
  {#snippet trigger({ props })}
    <button
      {...props}
      class="person"
      class:strong
      type="button"
      title={person.address}
      aria-label={label}
    >
      <span class="name">{name}</span>
      {#if person.contact !== null}
        <span class="badge" title={m.person_in_contacts()}>
          <BookUserIcon size={12} aria-hidden="true" />
        </span>
      {/if}
    </button>
  {/snippet}
</CommandMenu>

<style>
  .person {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    max-width: 100%;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .person:hover .name {
    text-decoration: underline;
  }

  .person.strong {
    font-weight: var(--font-weight-bold);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge {
    display: inline-flex;
    color: var(--color-accent);
  }
</style>
