<script lang="ts" generics="C">
  import LogInIcon from "@lucide/svelte/icons/log-in";
  import KeyRoundIcon from "@lucide/svelte/icons/key-round";
  import { m } from "../lib/paraglide/messages.js";
  import { folderLabel, folderServerName } from "../lib/folder-label";
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
  import LoaderIcon from "@lucide/svelte/icons/loader-circle";
  import SquarePenIcon from "@lucide/svelte/icons/square-pen";
  import SettingsIcon from "@lucide/svelte/icons/settings";
  import RefreshIcon from "@lucide/svelte/icons/refresh-cw";
  import OutboxIcon from "@lucide/svelte/icons/send-horizontal";
  import EllipsisIcon from "@lucide/svelte/icons/ellipsis";
  import FolderPlusIcon from "@lucide/svelte/icons/folder-plus";
  import PencilIcon from "@lucide/svelte/icons/pencil";
  import FolderXIcon from "@lucide/svelte/icons/folder-x";
  import FolderInputIcon from "@lucide/svelte/icons/folder-input";
  import ContactIcon from "@lucide/svelte/icons/book-user";
  import { isEditableFolder } from "../lib/folder-edit.svelte";
  import { folderPayload, type DragMove } from "../lib/drag-move.svelte";
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
    /** Comptes en cours de synchronisation : dossiers traités sur le total. */
    syncing?: Record<number, { done: number; total: number }>;
    /** Comptes OAuth à reconnecter (bouton visible sur la ligne). */
    reauth?: Record<number, true>;
    /** Comptes dont Google refuse le mot de passe d'application. */
    appPassword?: Record<number, true>;
    scope: ScopeDto;
    selectedId: number | null;
    onScope: (scope: ScopeDto, account: number | null) => void;
    onAdd: () => void;
    registry: CommandRegistry<C>;
    context: C;
    /** Sélectionne un compte ; renvoie le contexte des commandes à jour. */
    onAccount: (id: number) => C;
    /** Contexte des commandes qui visent le dossier `folder` du compte
     * `account` ; `target` : le dossier devient la cible des commandes
     * (menu du dossier choisi). */
    onFolder: (account: number, folder: FolderDto, target: boolean) => C;
    /** Glisser-déposer en cours : dossier ou premier niveau survolés ;
     * un dossier ordinaire peut lui-même être glissé. */
    drag?: DragMove;
    /** Le carnet d'adresses est affiché (aucun dossier n'est alors le
     * dossier courant). */
    contactsActive?: boolean;
  }

  let {
    accounts,
    folders,
    offline,
    syncing = {},
    reauth = {},
    appPassword = {},
    scope,
    selectedId,
    onScope,
    onAdd,
    registry,
    context,
    onAccount,
    onFolder,
    drag,
    contactsActive = false,
  }: Props = $props();

  /** Bouton de la souris enfoncé sur un dossier : début possible d'un
   * glisser (dossiers ordinaires seulement). */
  function press(e: PointerEvent, account: number, folder: FolderDto): void {
    if (drag === undefined || e.button !== 0) return;
    if (e.pointerType !== "mouse" && e.pointerType !== "pen") return;
    const payload = folderPayload(account, folder);
    if (payload !== null) drag.press(payload, e.clientX, e.clientY);
  }

  function dragged(folder: FolderDto): boolean {
    return drag?.payload?.kind === "folder" && drag.payload.folder.id === folder.id;
  }

  // Le registre n'est pas réactif : relecture à chaque modification.
  const watch = new RegistryWatch(() => registry);

  function describe(id: string, label?: string) {
    void watch.version;
    return describeCommand(registry, id, context, label);
  }

  const write = $derived(describe("compose.new"));
  const help = $derived(describe("help.shortcuts"));
  const settings = $derived(describe("settings.open", m.sidebar_settings()));
  const contacts = $derived(describe("contacts.open", m.sidebar_contacts()));

  /** Menu d'un compte : actions qui visent le compte sélectionné. */
  const accountActions: VisibleAction[] = [
    { command: "account.sync", icon: RefreshIcon, label: m.sidebar_sync, section: 0 },
    { command: "account.reconnect", icon: LogInIcon, label: m.sidebar_reconnect, section: 0 },
    {
      command: "account.change-password",
      icon: KeyRoundIcon,
      label: m.sidebar_change_password,
      section: 0,
    },
    { command: "drafts.open", icon: FileIcon, section: 0 },
    { command: "compose.outbox", icon: OutboxIcon, section: 0 },
    { command: "account.new-folder", icon: FolderPlusIcon, section: 1 },
    { command: "account.archive-folder", icon: ArchiveIcon, section: 1 },
    { command: "account.remove", icon: TrashIcon, label: m.sidebar_remove, section: 2 },
  ];

  /** Menu d'un dossier ordinaire. */
  const folderActions: VisibleAction[] = [
    { command: "folder.rename", icon: PencilIcon, section: 0 },
    { command: "folder.move", icon: FolderInputIcon, section: 0 },
    { command: "folder.delete", icon: FolderXIcon, section: 1 },
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

  const unifiedSelected = $derived(
    !contactsActive && scope.kind === "unified" && scope.role === "inbox",
  );
  const totalUnread = $derived(
    Object.values(folders)
      .flat()
      .filter((f) => f.role === "inbox")
      .reduce((n, f) => n + f.unread, 0),
  );

  function serverNameTitle(f: FolderDto): string | undefined {
    const name = folderServerName(f);
    return name === undefined ? undefined : m.mailbox_server_name({ name });
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
          <div
            class="account"
            class:current={account.id === selectedId}
            class:drop-over={drag?.overRoot === account.id}
            data-drop-account={account.id}
          >
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
              actions={accountActions.filter(
                (a) =>
                  a.command !==
                  (account.oauth_provider === null
                    ? "account.reconnect"
                    : "account.change-password"),
              )}
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
          {#if syncing[account.id]}
            {@const progress = syncing[account.id]}
            <div
              class="syncing"
              role="status"
              aria-label={m.sidebar_syncing_label({
                address: account.address,
                done: progress?.done ?? 0,
                total: progress?.total ?? 0,
              })}
              title={m.sidebar_syncing_label({
                address: account.address,
                done: progress?.done ?? 0,
                total: progress?.total ?? 0,
              })}
            >
              <LoaderIcon size={14} class="spin" aria-hidden="true" />
              <span
                >{m.sidebar_syncing({
                  done: progress?.done ?? 0,
                  total: progress?.total ?? 0,
                })}</span
              >
            </div>
          {/if}
          {#if reauth[account.id]}
            <div class="reauth" role="status">
              <span>{m.sidebar_reauth_hint()}</span>
              <button
                class="btn reconnect"
                type="button"
                title={m.sidebar_reauth_required({ address: account.address })}
                onclick={() => void registry.execute("account.reconnect", onAccount(account.id))}
              >
                <LogInIcon size={14} aria-hidden="true" />
                {m.sidebar_reconnect()}
              </button>
            </div>
          {/if}
          {#if appPassword[account.id]}
            <div class="reauth" role="status">
              <span>{m.sidebar_app_password_hint()}</span>
              <button
                class="btn reconnect"
                type="button"
                title={m.sidebar_app_password_required({ address: account.address })}
                onclick={() =>
                  void registry.execute("account.change-password", onAccount(account.id))}
              >
                <KeyRoundIcon size={14} aria-hidden="true" />
                {m.sidebar_change_password()}
              </button>
            </div>
          {/if}
          <ul class="folders" aria-label={m.sidebar_folders_of({ address: account.address })}>
            {#each folders[account.id] ?? [] as f (f.id)}
              {@const Icon = icon(f.role)}
              {@const current = !contactsActive && scope.kind === "mailbox" && scope.id === f.id}
              <li
                class:selected={current}
                class:editable={isEditableFolder(f)}
                class:drop-over={drag?.over === f.id}
                class:dragged={dragged(f)}
              >
                <button
                  class="folder"
                  type="button"
                  data-drop-folder={f.id}
                  data-drop-account={account.id}
                  disabled={!f.selectable}
                  aria-current={current ? "true" : undefined}
                  title={serverNameTitle(f)}
                  style:padding-left="calc(var(--space-2) + {Math.min(f.depth, 6) * 12}px)"
                  onpointerdown={(e) => press(e, account.id, f)}
                  onclick={() => {
                    // Clic qui termine un glisser : le dossier n'est pas affiché.
                    if (drag?.suppressClick()) return;
                    onScope({ kind: "mailbox", id: f.id }, account.id);
                  }}
                >
                  <Icon size={16} aria-hidden="true" />
                  <span class="label">{folderLabel(f)}</span>
                  {#if f.unread > 0}<span class="unread">{f.unread}</span>{/if}
                </button>
                {#if isEditableFolder(f)}
                  <CommandMenu
                    actions={folderActions}
                    {registry}
                    context={onFolder(account.id, f, false)}
                    label={m.sidebar_folder_menu({ folder: folderLabel(f) })}
                    run={(id) => void registry.execute(id, onFolder(account.id, f, true))}
                    align="start"
                  >
                    {#snippet trigger({ props })}
                      <button
                        {...props}
                        class="btn btn-ghost icon folder-menu"
                        type="button"
                        title={m.sidebar_folder_menu({ folder: folderLabel(f) })}
                        aria-label={m.sidebar_folder_menu({ folder: folderLabel(f) })}
                      >
                        <EllipsisIcon size={16} aria-hidden="true" />
                      </button>
                    {/snippet}
                  </CommandMenu>
                {/if}
              </li>
            {/each}
          </ul>
        </li>
      {/each}
    </ul>
  {/if}

  <ul class="folders footer">
    <li class:selected={contactsActive}>
      <button
        class="folder"
        type="button"
        title={contacts?.tooltip}
        aria-current={contactsActive ? "true" : undefined}
        onclick={() => void registry.execute("contacts.open", context)}
      >
        <ContactIcon size={16} aria-hidden="true" />
        <span class="label">{contacts?.label}</span>
      </button>
    </li>
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

  .syncing {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 var(--space-2) var(--space-1);
    padding: 0 var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .syncing :global(.spin) {
    animation: sidebar-spin 1s linear infinite;
  }

  @keyframes sidebar-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .syncing :global(.spin) {
      animation: none;
    }
  }

  .reauth {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: 0 var(--space-2) var(--space-2);
    padding: var(--space-2);
    border-radius: var(--radius);
    background: var(--color-warning-bg);
    color: var(--color-warning);
    font-size: var(--font-size-sm);
  }

  .reconnect {
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-sm);
    gap: var(--space-1);
  }

  .offline {
    color: var(--color-warning);
  }

  .folders li {
    border-radius: var(--radius);
  }

  /* Dossier ordinaire : son menu « ⋯ » à droite du nom. */
  .folders li.editable {
    display: flex;
    align-items: center;
  }

  /* Menu visible au survol, au clavier, sur le dossier affiché et tant
   * qu'il est ouvert ; sa place est toujours réservée. */
  .folder-menu {
    flex: none;
    margin-right: var(--space-1);
    opacity: 0;
  }

  .folders li:hover .folder-menu,
  .folders li:focus-within .folder-menu,
  .folders li.selected .folder-menu,
  .folder-menu[aria-expanded="true"] {
    opacity: 1;
  }

  .folders li:hover {
    background: var(--color-hover);
  }

  .folders li.selected {
    background: var(--color-selected);
  }

  /* Dossier (ou premier niveau d'un compte) où ce qui est glissé sera
     déposé. */
  .folders li.drop-over,
  .account.drop-over {
    background: var(--color-selected);
    outline: var(--border-width) solid var(--color-accent);
  }

  .folders li.dragged {
    opacity: 0.5;
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
