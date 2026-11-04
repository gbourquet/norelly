<script lang="ts">
  import { m } from "./lib/paraglide/messages.js";
  import { errorText } from "./lib/errors";
  import { onMount } from "svelte";
  import InboxIcon from "@lucide/svelte/icons/inbox";
  import ThreePaneLayout from "./components/ThreePaneLayout.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import CommandPalette from "./components/CommandPalette.svelte";
  import ShortcutHelp from "./components/ShortcutHelp.svelte";
  import AddAccountDialog from "./components/AddAccountDialog.svelte";
  import RemoveAccountDialog from "./components/RemoveAccountDialog.svelte";
  import ConversationList from "./components/ConversationList.svelte";
  import ConversationView from "./components/ConversationView.svelte";
  import FolderPicker from "./components/FolderPicker.svelte";
  import ActionNotice from "./components/ActionNotice.svelte";
  import ActionButtons from "./components/ActionButtons.svelte";
  import ComposeDialog from "./components/ComposeDialog.svelte";
  import OutboxDialog from "./components/OutboxDialog.svelte";
  import SearchPanel from "./components/SearchPanel.svelte";
  import SettingsDialog from "./components/SettingsDialog.svelte";
  import DraftsDialog from "./components/DraftsDialog.svelte";
  import UpdateQuestion from "./components/UpdateQuestion.svelte";
  import { notificationText } from "./lib/notifications";
  import { conversationActions } from "./lib/visible-actions";
  import {
    app,
    compose,
    mail,
    moveAccount,
    openSelectedHit,
    registerAppCommands,
    registry,
    search,
    settings,
  } from "./lib/app.svelte";
  import { shortcutFromEvent, type Shortcut } from "./lib/commands/shortcut";
  import { api, type ScopeDto } from "./lib/ipc/api";
  import { UNIFIED_INBOX } from "./lib/mail.svelte";

  onMount(() => {
    performance.mark("norelly:mounted");
    requestAnimationFrame(() => performance.mark("norelly:first-frame"));
    setTimeout(() => performance.mark("norelly:first-task"), 0);
    const unregister = registerAppCommands();
    let unlisten: (() => void) | null = null;
    let unlistenCompose: (() => void) | null = null;
    let disposed = false;
    void settings.load();
    void (async () => {
      // Comptes et première liste en parallèle : la boîte unifiée ne
      // dépend pas de la liste des comptes (démarrage plus court).
      await Promise.all([
        app.refreshAccounts().then(() => performance.mark("norelly:accounts-read")),
        mail.show(UNIFIED_INBOX),
      ]);
      try {
        const off = await api.onSyncEvent((event) => {
          void mail.onSyncEvent(event);
          // Résultats affichés tenus à jour (index local modifié).
          if (event.kind === "folder_changed") void search.refresh();
          // Nouveaux messages : notification si la fenêtre n'a pas le
          // focus (Rust a déjà appliqué les paramètres).
          if (event.kind === "new_mail" && !document.hasFocus()) {
            const account = app.accounts.find((a) => a.id === event.account);
            const { title, body } = notificationText(event, account?.address ?? "");
            void api.showNotification(title, body).catch(() => undefined);
          }
        });
        if (disposed) off();
        else unlisten = off;
        const offCompose = await api.onComposeEvent((event) => {
          if (event.kind === "outbox_changed") void app.refreshOutbox(event.account);
          else if (compose.draft !== null) void compose.attachDropped(event.token);
        });
        if (disposed) offCompose();
        else unlistenCompose = offCompose;
      } catch {
        // Hors de Tauri (tests) : pas d'événements.
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
      unlistenCompose?.();
      unregister();
    };
  });

  const listTitle = $derived.by(() => {
    const scope = mail.scope;
    if (scope.kind === "unified") return m.mailbox_unified_inbox();
    for (const folders of Object.values(mail.folders)) {
      const f = folders.find((x) => x.id === scope.id);
      if (f !== undefined) return f.role === "inbox" ? m.mailbox_inbox() : f.leaf;
    }
    return m.mailbox_folder();
  });

  /** Dossiers proposés pour « déplacer » : ceux du compte visé, sauf ceux
   * où se trouvent déjà les conversations visées. */
  const moveFolders = $derived.by(() => {
    if (app.dialog !== "move") return [];
    const account = moveAccount();
    const current = mail.targets().map((c) => c.mailbox);
    return account === null
      ? []
      : (mail.folders[account] ?? []).filter((f) => !current.includes(f.id));
  });

  const composeTitle = $derived(
    app.composeKind === "new"
      ? m.compose_title_new()
      : app.composeKind === "forward"
        ? m.compose_title_forward()
        : m.compose_title_reply(),
  );

  const archiveFolders = $derived(
    app.selectedAccountId === null ? [] : (mail.folders[app.selectedAccountId] ?? []),
  );

  function selectScope(scope: ScopeDto, account: number | null): void {
    if (account !== null) app.selectedAccountId = account;
    search.close();
    void mail.show(scope);
  }

  /** Touche reçue de l'agent d'un message (focus dans le corps). */
  function onFrameKey(shortcut: Shortcut): void {
    if (app.dialog !== null) return;
    const command = registry.resolve(shortcut, app.context(), false);
    if (command !== undefined) void registry.execute(command.id, app.context());
  }

  function isEditable(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;
    return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
  }

  function onKeyDown(e: KeyboardEvent): void {
    // Les fenêtres modales gèrent leur propre clavier.
    if (app.dialog !== null || e.isComposing || e.defaultPrevented) return;
    const command = registry.resolve(shortcutFromEvent(e), app.context(), isEditable(e.target));
    if (command !== undefined) {
      e.preventDefault();
      void registry.execute(command.id, app.context());
    }
  }
</script>

<svelte:window onkeydown={onKeyDown} />

<ThreePaneLayout>
  {#snippet sidebar()}
    <Sidebar
      accounts={app.accounts}
      folders={mail.folders}
      offline={mail.offline}
      scope={mail.scope}
      selectedId={app.selectedAccountId}
      onScope={selectScope}
      onAdd={() => app.open("add-account")}
      onRemove={(id) => {
        app.selectedAccountId = id;
        app.open("remove-account");
      }}
    />
  {/snippet}
  {#snippet list()}
    {#if search.active}
      <SearchPanel {search} onOpen={() => void openSelectedHit()} />
    {:else if app.accounts.length > 0}
      <ConversationList
        {mail}
        title={listTitle}
        {registry}
        context={app.context()}
        onSearch={() => search.open()}
      />
    {/if}
  {/snippet}
  {#snippet reader()}
    {#if mail.opened !== null}
      <ConversationView
        conversation={mail.opened}
        messages={mail.messages}
        {api}
        onBack={() => mail.back()}
        onKey={onFrameKey}
      >
        {#snippet toolbar()}
          <ActionButtons
            actions={conversationActions(mail.opened)}
            {registry}
            context={app.context()}
            label={m.conversation_toolbar()}
          />
        {/snippet}
      </ConversationView>
    {:else}
      <div class="placeholder welcome">
        <InboxIcon size={40} aria-hidden="true" />
        <h1>{m.app_name()}</h1>
        {#if app.loadError}
          <p role="alert">{m.welcome_load_error({ error: app.loadError })}</p>
        {:else if app.accounts.length === 0}
          <p>{m.welcome_no_account()}</p>
          <button class="btn btn-primary" type="button" onclick={() => app.open("add-account")}
            >{m.welcome_add_account()}</button
          >
        {/if}
        <p class="hint">
          {m.welcome_hint_palette()} <kbd>Ctrl</kbd> <kbd>K</kbd> · {m.welcome_hint_shortcuts()}
          <kbd>?</kbd>
        </p>
      </div>
    {/if}
  {/snippet}
</ThreePaneLayout>

<CommandPalette
  open={app.dialog === "palette"}
  {registry}
  context={app.context()}
  onClose={() => app.close()}
/>
<ShortcutHelp open={app.dialog === "help"} {registry} onClose={() => app.close()} />
<AddAccountDialog
  open={app.dialog === "add-account"}
  {api}
  onClose={() => app.close()}
  onAdded={(account) => {
    app.close();
    app.selectedAccountId = account.id;
    void app.refreshAccounts().then(() => mail.reload());
  }}
/>
<FolderPicker
  open={app.dialog === "move"}
  title={m.picker_move_title()}
  folders={moveFolders}
  onClose={() => app.close()}
  onChoose={(folder) => {
    app.close();
    void mail.act({ kind: "move", mailbox: folder.id }, m.action_moved({ folder: folder.name }));
  }}
/>
<FolderPicker
  open={app.dialog === "archive-folder"}
  title={m.picker_archive_title()}
  folders={archiveFolders}
  onClose={() => app.close()}
  onChoose={(folder) => {
    app.close();
    const account = app.selectedAccountId;
    if (account === null) return;
    api.setArchiveFolder(account, folder.id).then(
      () =>
        (mail.notice = {
          text: m.action_archive_folder_set({ folder: folder.name }),
          undoable: false,
        }),
      (e: unknown) => (mail.notice = { text: errorText(e), undoable: false }),
    );
  }}
/>
<ComposeDialog
  open={app.dialog === "compose"}
  title={composeTitle}
  {compose}
  onClose={() => {
    void compose.close();
    app.close();
  }}
  onSent={(scheduled) => {
    app.close();
    mail.notice = {
      text: scheduled ? m.compose_scheduled() : m.compose_queued(),
      undoable: false,
    };
  }}
/>
<DraftsDialog
  open={app.dialog === "drafts"}
  drafts={app.drafts}
  onClose={() => app.close()}
  onOpen={(id) => void app.openDraft(id)}
  onResume={(message) => {
    const account = app.draftsAccount;
    if (account !== null) void app.resumeRemoteDraft(account, message);
  }}
/>
<OutboxDialog
  open={app.dialog === "outbox"}
  items={app.outbox}
  onClose={() => app.close()}
  onEdit={(id) => {
    const account = app.outboxAccount;
    if (account !== null) void app.editScheduled(account, id);
  }}
  onRetry={(id) => {
    const account = app.outboxAccount;
    if (account === null) return;
    api.retrySend(account, id).then(
      () => app.refreshOutbox(account),
      (e: unknown) => (mail.notice = { text: errorText(e), undoable: false }),
    );
  }}
  onCancel={(id) => {
    const account = app.outboxAccount;
    if (account === null) return;
    api.cancelSend(account, id).then(
      () => app.refreshOutbox(account),
      (e: unknown) => (mail.notice = { text: errorText(e), undoable: false }),
    );
  }}
/>
<ActionNotice
  notice={mail.notice}
  canUndo={mail.lastAction !== null}
  onUndo={() => void mail.undo()}
  onDismiss={() => (mail.notice = null)}
/>
<SettingsDialog
  open={app.dialog === "settings"}
  settings={settings.current}
  plainText={app.plainText}
  error={settings.error}
  onClose={() => app.close()}
  onSave={async (next, plain) => {
    const saved = await settings.save(next);
    if (!saved) return;
    if (plain !== app.plainText) await api.setPreferPlainText(plain).catch(() => undefined);
    app.close();
  }}
/>
<UpdateQuestion
  open={settings.askUpdate && app.dialog === null}
  onAnswer={(yes) => void settings.answerUpdate(yes)}
/>
<RemoveAccountDialog
  open={app.dialog === "remove-account"}
  account={app.selectedAccount}
  onClose={() => app.close()}
  onConfirm={async (account) => {
    await api.removeAccount(account.id);
    app.close();
    mail.forgetAccount(account.id);
    app.selectedAccountId = null;
    await app.refreshAccounts();
    await mail.reload();
  }}
/>

<style>
  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    height: 100%;
    padding: var(--space-6);
    color: var(--color-text-muted);
    text-align: center;
  }

  .welcome h1 {
    margin: 0;
    font-size: var(--font-size-xl);
    font-weight: var(--font-weight-medium);
  }

  .hint {
    font-size: var(--font-size-sm);
  }
</style>
