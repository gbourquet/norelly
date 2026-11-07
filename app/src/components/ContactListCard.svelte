<script lang="ts" generics="C">
  import { m } from "../lib/paraglide/messages.js";
  // Liste de diffusion ouverte, en lecture : barre d'actions (écrire,
  // modifier, supprimer), description, envoi en Cci, membres avec
  // l'adresse préférée à laquelle chacun sera écrit. Tout est affiché en
  // texte.
  import UsersIcon from "@lucide/svelte/icons/users";
  import EyeOffIcon from "@lucide/svelte/icons/eye-off";
  import type { ContactList } from "../lib/ipc/api";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { contactListToolbar } from "../lib/visible-actions";
  import ActionBar from "./ActionBar.svelte";

  interface Props {
    list: ContactList;
    registry: CommandRegistry<C>;
    context: C;
  }

  let { list, registry, context }: Props = $props();

  const actions = contactListToolbar();
  const f = $derived(list.fields);
</script>

<article class="card" aria-label={m.lists_card({ name: f.name })}>
  <div class="toolbar">
    <ActionBar
      primary={actions.primary}
      more={actions.more}
      {registry}
      {context}
      label={m.lists_card_toolbar()}
    />
  </div>
  <div class="body">
    <header>
      <div class="avatar" aria-hidden="true"><UsersIcon size={28} /></div>
      <div class="titles">
        <h2>{f.name}</h2>
        <p class="sub">{m.lists_members_count({ count: list.members.length })}</p>
      </div>
    </header>

    {#if f.description !== ""}
      <p class="description">{f.description}</p>
    {/if}

    {#if f.bcc}
      <p class="notice">
        <EyeOffIcon size={16} aria-hidden="true" />
        <span>{m.lists_bcc_explained()}</span>
      </p>
    {/if}

    <section aria-label={m.lists_members()}>
      <h3>{m.lists_members()}</h3>
      {#if list.members.length === 0}
        <p class="empty">{m.lists_no_members()}</p>
      {:else}
        <p class="hint">{m.lists_preferred_explained()}</p>
        <ul>
          {#each list.members as member (member.id)}
            <li>
              <span class="name">{member.name}</span>
              {#if member.name !== member.address}
                <span class="address">{member.address}</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>
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
  }

  .titles {
    min-width: 0;
  }

  h2 {
    margin: 0;
    font-size: var(--font-size-xl);
    font-weight: var(--font-weight-medium);
    overflow-wrap: anywhere;
  }

  .sub,
  .hint,
  .empty {
    margin: var(--space-1) 0 0;
    color: var(--color-text-muted);
  }

  .hint {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-sm);
  }

  .description {
    margin: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .notice {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-surface);
  }

  h3 {
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
    align-items: baseline;
    gap: var(--space-2);
  }

  .name {
    font-weight: var(--font-weight-medium);
    overflow-wrap: anywhere;
  }

  .address {
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
    user-select: text;
  }
</style>
