<script lang="ts">
  // Panneau de navigation : boîte de réception unifiée, puis comptes et
  // leurs dossiers.
  import InboxIcon from "@lucide/svelte/icons/inbox";
  import FolderIcon from "@lucide/svelte/icons/folder";
  import SendIcon from "@lucide/svelte/icons/send";
  import FileIcon from "@lucide/svelte/icons/file-pen";
  import ArchiveIcon from "@lucide/svelte/icons/archive";
  import TrashIcon from "@lucide/svelte/icons/trash-2";
  import BanIcon from "@lucide/svelte/icons/ban";
  import MailIcon from "@lucide/svelte/icons/mail";
  import PlusIcon from "@lucide/svelte/icons/plus";
  import WifiOffIcon from "@lucide/svelte/icons/wifi-off";
  import type { AccountDto, FolderDto, ScopeDto } from "../lib/ipc/api";

  interface Props {
    accounts: AccountDto[];
    folders: Record<number, FolderDto[]>;
    offline: Record<number, string>;
    scope: ScopeDto;
    selectedId: number | null;
    onScope: (scope: ScopeDto, account: number | null) => void;
    onAdd: () => void;
    onRemove: (id: number) => void;
  }

  let { accounts, folders, offline, scope, selectedId, onScope, onAdd, onRemove }: Props = $props();

  const ICONS = {
    inbox: InboxIcon,
    sent: SendIcon,
    drafts: FileIcon,
    archive: ArchiveIcon,
    trash: TrashIcon,
    junk: BanIcon,
  } as const;

  function icon(role: string): typeof FolderIcon {
    return role in ICONS ? ICONS[role as keyof typeof ICONS] : FolderIcon;
  }

  const unifiedSelected = $derived(scope.kind === "unified" && scope.role === "inbox");
  const totalUnread = $derived(
    Object.values(folders)
      .flat()
      .filter((f) => f.role === "inbox")
      .reduce((n, f) => n + f.unread, 0),
  );

  function folderLabel(f: FolderDto): string {
    return f.role === "inbox" ? "Boîte de réception" : f.leaf;
  }
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
    <ul class="folders" aria-label="Boîtes unifiées">
      <li class:selected={unifiedSelected}>
        <button
          class="folder"
          type="button"
          aria-current={unifiedSelected ? "true" : undefined}
          onclick={() => onScope({ kind: "unified", role: "inbox" }, null)}
        >
          <InboxIcon size={16} aria-hidden="true" />
          <span class="label">Boîte de réception unifiée</span>
          {#if totalUnread > 0}<span class="unread">{totalUnread}</span>{/if}
        </button>
      </li>
    </ul>
    <ul class="accounts" aria-label="Comptes">
      {#each accounts as account (account.id)}
        <li>
          <div class="account" class:current={account.id === selectedId}>
            <MailIcon size={16} aria-hidden="true" />
            <span class="labels">
              <span class="name">{account.display_name}</span>
              <span class="address">{account.address}</span>
            </span>
            {#if offline[account.id]}
              <span class="offline" title="Hors ligne : {offline[account.id]}">
                <WifiOffIcon size={14} aria-label="Hors ligne" />
              </span>
            {/if}
            <button
              class="btn btn-ghost icon remove"
              type="button"
              title="Supprimer le compte {account.address}"
              aria-label="Supprimer le compte {account.address}"
              onclick={() => onRemove(account.id)}
            >
              <TrashIcon size={14} aria-hidden="true" />
            </button>
          </div>
          <ul class="folders" aria-label="Dossiers de {account.address}">
            {#each folders[account.id] ?? [] as f (f.id)}
              {@const Icon = icon(f.role)}
              {@const current = scope.kind === "mailbox" && scope.id === f.id}
              <li class:selected={current}>
                <button
                  class="folder"
                  type="button"
                  disabled={!f.selectable}
                  aria-current={current ? "true" : undefined}
                  style:padding-left="calc(var(--space-2) + {Math.min(f.depth, 6) * 12}px)"
                  onclick={() => onScope({ kind: "mailbox", id: f.id }, account.id)}
                >
                  <Icon size={16} aria-hidden="true" />
                  <span class="label">{folderLabel(f)}</span>
                  {#if f.unread > 0}<span class="unread">{f.unread}</span>{/if}
                </button>
              </li>
            {/each}
          </ul>
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

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .account {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    margin-top: var(--space-2);
  }

  .labels {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .name,
  .address,
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name {
    font-weight: var(--font-weight-medium);
  }

  .address {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .offline {
    color: var(--color-warning);
  }

  .remove {
    opacity: 0;
  }

  .account:hover .remove,
  .account.current .remove,
  .remove:focus-visible {
    opacity: 1;
  }

  .folders li {
    border-radius: var(--radius);
  }

  .folders li:hover {
    background: var(--color-hover);
  }

  .folders li.selected {
    background: var(--color-selected);
  }

  .folder {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-width: 0;
    padding: var(--space-1) var(--space-2);
    border: none;
    background: transparent;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }

  .folder:disabled {
    color: var(--color-text-muted);
    cursor: default;
  }

  .label {
    flex: 1;
  }

  .unread {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-bold);
  }
</style>
