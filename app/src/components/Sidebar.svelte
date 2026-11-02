<script lang="ts">
  // Panneau de navigation : comptes (les dossiers arrivent en phase 4).
  import MailIcon from "@lucide/svelte/icons/mail";
  import PlusIcon from "@lucide/svelte/icons/plus";
  import TrashIcon from "@lucide/svelte/icons/trash-2";
  import type { AccountDto } from "../lib/ipc/api";

  interface Props {
    accounts: AccountDto[];
    selectedId: number | null;
    onSelect: (id: number) => void;
    onAdd: () => void;
    onRemove: (id: number) => void;
  }

  let { accounts, selectedId, onSelect, onAdd, onRemove }: Props = $props();
</script>

<div class="sidebar">
  <header>
    <span class="brand">Norelly</span>
    <button
      class="btn btn-ghost icon"
      type="button"
      title="Ajouter un compte"
      aria-label="Ajouter un compte"
      onclick={onAdd}
    >
      <PlusIcon size={16} aria-hidden="true" />
    </button>
  </header>

  {#if accounts.length === 0}
    <p class="empty">Aucun compte.</p>
  {:else}
    <ul class="accounts" aria-label="Comptes">
      {#each accounts as account (account.id)}
        <li class:selected={account.id === selectedId}>
          <button
            class="account"
            type="button"
            aria-current={account.id === selectedId ? "true" : undefined}
            onclick={() => onSelect(account.id)}
          >
            <MailIcon size={16} aria-hidden="true" />
            <span class="labels">
              <span class="name">{account.display_name}</span>
              <span class="address">{account.address}</span>
            </span>
          </button>
          <button
            class="btn btn-ghost icon remove"
            type="button"
            title="Supprimer le compte {account.address}"
            aria-label="Supprimer le compte {account.address}"
            onclick={() => onRemove(account.id)}
          >
            <TrashIcon size={14} aria-hidden="true" />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-2);
  }

  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 var(--space-2);
  }

  .brand {
    font-weight: var(--font-weight-bold);
  }

  .icon {
    padding: var(--space-1);
    min-height: 0;
  }

  .empty {
    padding: 0 var(--space-2);
    color: var(--color-text-muted);
  }

  .accounts {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    border-radius: var(--radius);
  }

  li:hover {
    background: var(--color-hover);
  }

  li.selected {
    background: var(--color-selected);
  }

  .account {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-2);
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .labels {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .name,
  .address {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .address {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .remove {
    opacity: 0;
  }

  li:hover .remove,
  li.selected .remove,
  .remove:focus-visible {
    opacity: 1;
  }
</style>
