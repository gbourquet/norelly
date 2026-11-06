<script lang="ts" generics="C">
  import { m } from "../lib/paraglide/messages.js";
  // Panneau de navigation : « Nouveau message », boîte de réception
  // unifiée, comptes et leurs dossiers (chacun avec son menu d'actions),
  // puis ajout de compte et paramètres en bas.
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
  import SquarePenIcon from "@lucide/svelte/icons/square-pen";
  import SettingsIcon from "@lucide/svelte/icons/settings";
  import RefreshIcon from "@lucide/svelte/icons/refresh-cw";
  import OutboxIcon from "@lucide/svelte/icons/send-horizontal";
  import EllipsisIcon from "@lucide/svelte/icons/ellipsis";
  import FolderPlusIcon from "@lucide/svelte/icons/folder-plus";
  import type { AccountDto, FolderDto, ScopeDto } from "../lib/ipc/api";
  import type { CommandRegistry } from "../lib/commands/registry";
  import { RegistryWatch } from "../lib/commands/watch.svelte";
  import { describeCommand } from "../lib/commands/describe";
  import type { VisibleAction } from "../lib/visible-actions";
  import CommandMenu from "./CommandMenu.svelte";

  interface Props {
    accounts: AccountDto[];
    folders: Record<number, FolderDto[]>;
    offline: Record<number, string>;
    scope: ScopeDto;
    selectedId: number | null;
    onScope: (scope: ScopeDto, account: number | null) => void;
    onAdd: () => void;
    registry: CommandRegistry<C>;
    context: C;
    /** Sélectionne un compte ; renvoie le contexte des commandes à jour. */
    onAccount: (id: number) => C;
  }

  let {
    accounts,
    folders,
    offline,
    scope,
    selectedId,
    onScope,
    onAdd,
    registry,
    context,
    onAccount,
  }: Props = $props();

  // Le registre n'est pas réactif : relecture à chaque modification.
  const watch = new RegistryWatch(() => registry);

  function describe(id: string, label?: string) {
    void watch.version;
    return describeCommand(registry, id, context, label);
  }

  const write = $derived(describe("compose.new"));
  const help = $derived(describe("help.shortcuts"));
  const settings = $derived(describe("settings.open", m.sidebar_settings()));

  /** Menu d'un compte : actions qui visent le compte sélectionné. */
  const accountActions: VisibleAction[] = [
    { command: "account.sync", icon: RefreshIcon, label: m.sidebar_sync, section: 0 },
    { command: "drafts.open", icon: FileIcon, section: 0 },
    { command: "compose.outbox", icon: OutboxIcon, section: 0 },
    { command: "account.new-folder", icon: FolderPlusIcon, section: 1 },
    { command: "account.archive-folder", icon: ArchiveIcon, section: 1 },
    { command: "account.remove", icon: TrashIcon, label: m.sidebar_remove, section: 2 },
  ];

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
    return f.role === "inbox" ? m.mailbox_inbox() : f.leaf;
  }
</script>

<div class="sidebar">
  <header>
    <span class="brand">{m.app_name()}</span>
    <button
      class="btn help"
      type="button"
      aria-label={help?.label}
      title={help?.tooltip}
      onclick={() => void registry.execute("help.shortcuts", context)}
    >
      <span aria-hidden="true">?</span>
    </button>
  </header>
  <button
    class="btn btn-primary write"
    type="button"
    title={write?.tooltip}
    disabled={!write?.enabled}
    onclick={() => void registry.execute("compose.new", context)}
  >
    <SquarePenIcon size={16} aria-hidden="true" />
    <span>{write?.label}</span>
  </button>

  {#if accounts.length === 0}
    <p class="empty">{m.sidebar_no_account()}</p>
  {:else}
    <ul class="folders" aria-label={m.sidebar_unified()}>
      <li class:selected={unifiedSelected}>
        <button
          class="folder"
          type="button"
          aria-current={unifiedSelected ? "true" : undefined}
          onclick={() => onScope({ kind: "unified", role: "inbox" }, null)}
        >
          <InboxIcon size={16} aria-hidden="true" />
          <span class="label">{m.mailbox_unified_inbox()}</span>
          {#if totalUnread > 0}<span class="unread">{totalUnread}</span>{/if}
        </button>
      </li>
    </ul>
    <ul class="accounts" aria-label={m.sidebar_accounts()}>
      {#each accounts as account (account.id)}
        <li>
          <div class="account" class:current={account.id === selectedId}>
            <MailIcon size={16} aria-hidden="true" />
            <span class="labels">
              <span class="name">{account.display_name}</span>
              <span class="address">{account.address}</span>
            </span>
            {#if offline[account.id]}
              <span
                class="offline"
                title={m.sidebar_offline_reason({ reason: offline[account.id] ?? "" })}
              >
                <WifiOffIcon size={14} aria-label={m.common_offline()} />
              </span>
            {/if}
            <CommandMenu
              actions={accountActions}
              {registry}
              {context}
              label={m.sidebar_account_menu({ address: account.address })}
              run={(id) => void registry.execute(id, onAccount(account.id))}
              align="start"
            >
              {#snippet trigger({ props })}
                <button
                  {...props}
                  class="btn btn-ghost icon"
                  type="button"
                  title={m.sidebar_account_menu({ address: account.address })}
                  aria-label={m.sidebar_account_menu({ address: account.address })}
                >
                  <EllipsisIcon size={16} aria-hidden="true" />
                </button>
              {/snippet}
            </CommandMenu>
          </div>
          <ul class="folders" aria-label={m.sidebar_folders_of({ address: account.address })}>
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

  <ul class="folders footer">
    <li>
      <button class="folder" type="button" title={m.sidebar_add_account()} onclick={onAdd}>
        <PlusIcon size={16} aria-hidden="true" />
        <span class="label">{m.sidebar_add_account()}</span>
      </button>
    </li>
    <li>
      <button
        class="folder"
        type="button"
        title={settings?.tooltip}
        onclick={() => void registry.execute("settings.open", context)}
      >
        <SettingsIcon size={16} aria-hidden="true" />
        <span class="label">{settings?.label}</span>
      </button>
    </li>
  </ul>
</div>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 100%;
    padding: var(--space-3) var(--space-2) 0;
  }

  .write {
    justify-content: center;
    margin: 0 var(--space-2) var(--space-2);
  }

  /* En bas du panneau, toujours visible. */
  .footer {
    position: sticky;
    bottom: 0;
    margin-top: auto;
    padding: var(--space-2) 0;
    border-top: var(--border-width) solid var(--color-border);
    background: var(--color-surface);
  }

  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 var(--space-2);
  }

  .help {
    justify-content: center;
    min-width: var(--space-8);
    min-height: var(--space-8);
    padding: 0;
    border-radius: 50%;
    font-weight: var(--font-weight-bold);
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
  .address {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Noms de dossiers jamais tronqués : ils passent à la ligne. */
  .label {
    min-width: 0;
    overflow-wrap: anywhere;
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
