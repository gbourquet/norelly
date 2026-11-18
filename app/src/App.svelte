<script lang="ts">
  import { m } from "./lib/paraglide/messages.js";
  import { folderLabel } from "./lib/folder-label";
  import { errorText } from "./lib/errors";
  import { onMount, untrack } from "svelte";
  import InboxIcon from "@lucide/svelte/icons/inbox";
  import ThreePaneLayout from "./components/ThreePaneLayout.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import CommandPalette from "./components/CommandPalette.svelte";
  import ShortcutHelp from "./components/ShortcutHelp.svelte";
  import AddAccountDialog from "./components/AddAccountDialog.svelte";
  import RemoveAccountDialog from "./components/RemoveAccountDialog.svelte";
  import ChangePasswordDialog from "./components/ChangePasswordDialog.svelte";
  import ConversationList from "./components/ConversationList.svelte";
  import ConversationView from "./components/ConversationView.svelte";
  import FolderPicker from "./components/FolderPicker.svelte";
  import NewFolderDialog from "./components/NewFolderDialog.svelte";
  import { NewFolderState } from "./lib/new-folder.svelte";
  import RenameFolderDialog from "./components/RenameFolderDialog.svelte";
  import DeleteFolderDialog from "./components/DeleteFolderDialog.svelte";
  import MoveFolderDialog from "./components/MoveFolderDialog.svelte";
  import { DeleteFolderState, MoveFolderState, RenameFolderState } from "./lib/folder-edit.svelte";
  import { folderFailureText } from "./lib/folder-failure";
  import { relayFrameCommand } from "./lib/agent";
  import { DragMove, edgeScroll, openedPayload, type DropTarget } from "./lib/drag-move.svelte";
  import ActionNotice from "./components/ActionNotice.svelte";
  import ActionBar from "./components/ActionBar.svelte";
  import ComposeDialog from "./components/ComposeDialog.svelte";
  import OutboxDialog from "./components/OutboxDialog.svelte";
  import SearchPanel from "./components/SearchPanel.svelte";
  import SettingsDialog from "./components/SettingsDialog.svelte";
  import LabelDialog from "./components/LabelDialog.svelte";
  import LabelSettings from "./components/LabelSettings.svelte";
  import LabelChip from "./components/LabelChip.svelte";
  import { selectionChips } from "./lib/labels.svelte";
  import DraftsDialog from "./components/DraftsDialog.svelte";
  import UpdateQuestion from "./components/UpdateQuestion.svelte";
  import UpdateBanner from "./components/UpdateBanner.svelte";
  import ContactList from "./components/ContactList.svelte";
  import ContactCard from "./components/ContactCard.svelte";
  import ContactDialog from "./components/ContactDialog.svelte";
  import DeleteContactsDialog from "./components/DeleteContactsDialog.svelte";
  import ContactExportDialog from "./components/ContactExportDialog.svelte";
  import ContactDuplicatesDialog from "./components/ContactDuplicatesDialog.svelte";
  import ContactImportDialog from "./components/ContactImportDialog.svelte";
  import CardDavDialog from "./components/CardDavDialog.svelte";
  import ContactListCard from "./components/ContactListCard.svelte";
  import ContactListDialog from "./components/ContactListDialog.svelte";
  import AddToListDialog from "./components/AddToListDialog.svelte";
  import DeleteContactListDialog from "./components/DeleteContactListDialog.svelte";
  import UsersIcon from "@lucide/svelte/icons/users";
  import PersonMenu from "./components/PersonMenu.svelte";
  import BookUserIcon from "@lucide/svelte/icons/book-user";
  import { notificationText } from "./lib/notifications";
  import { readerActions } from "./lib/visible-actions";
  import { describeCommand } from "./lib/commands/describe";
  import {
    addToList,
    app,
    carddav,
    compose,
    contactDeletion,
    contactForm,
    contactDuplicates,
    contactExport,
    contactImport,
    contacts,
    listDeletion,
    listForm,
    labelMenu,
    labelSettings,
    labels as labelsStore,
    lists,
    mail,
    moveAccount,
    searchLabel,
    openSelectedHit,
    registerAppCommands,
    registry,
    runForPerson,
    search,
    settings,
    updates,
  } from "./lib/app.svelte";
  import { shortcutFromEvent, type Shortcut } from "./lib/commands/shortcut";
  import { api, type FolderDto, type ScopeDto } from "./lib/ipc/api";
  import { UNIFIED_INBOX } from "./lib/mail.svelte";

  onMount(() => {
    performance.mark("norelly:mounted");
    requestAnimationFrame(() => performance.mark("norelly:first-frame"));
    setTimeout(() => performance.mark("norelly:first-task"), 0);
    const unregister = registerAppCommands();
    let unlisten: (() => void) | null = null;
    let unlistenCompose: (() => void) | null = null;
    let unlistenUpdate: (() => void) | null = null;
    let disposed = false;
    void settings.load();
    void updates.load().catch(() => undefined);
    // Carnet distant relié : synchronisé au lancement, liste relue s'il a
    // apporté des changements.
    void carddav.background().then((report) => {
      if (report === null || report.pulled === 0) return;
      void contacts.load();
      if (app.view === "mail") void mail.reload();
    });
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
          if (event.kind === "folder_changed" || event.kind === "folders_changed") {
            void search.refresh();
          }
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
        const offUpdate = await api.onUpdateEvent((event) => updates.handle(event));
        if (disposed) offUpdate();
        else unlistenUpdate = offUpdate;
      } catch {
        // Hors de Tauri (tests) : pas d'événements.
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
      unlistenCompose?.();
      unlistenUpdate?.();
      unregister();
    };
  });

  const listTitle = $derived.by(() => {
    const scope = mail.scope;
    if (scope.kind === "unified") return m.mailbox_unified_inbox();
    for (const folders of Object.values(mail.folders)) {
      const f = folders.find((x) => x.id === scope.id);
      if (f !== undefined) return folderLabel(f);
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

  // Résultat inconnu d'une modification de dossier : liste relue.
  const reloadFolders = (account: number): Promise<void> => mail.loadFolders(account);
  const newFolder = new NewFolderState(api, reloadFolders);
  const newFolderAccount = $derived(app.dialog === "new-folder" ? app.newFolderAccountId() : null);

  // Formulaire vide (ou prérempli : sous-dossier, nom cherché dans
  // « Déplacer vers… ») à chaque ouverture.
  $effect(() => {
    if (app.dialog === "new-folder") {
      const preset = app.newFolderPreset;
      untrack(() => newFolder.reset(preset));
    }
  });

  const renameFolder = new RenameFolderState(api, reloadFolders);
  const moveFolder = new MoveFolderState(api, reloadFolders);
  const deleteFolder = new DeleteFolderState(api, reloadFolders);
  /** Dossier visé par le renommage, le déplacement ou la suppression
   * ouverts. */
  const editedFolder = $derived(
    app.dialog === "rename-folder" || app.dialog === "move-folder" || app.dialog === "delete-folder"
      ? app.targetFolder()
      : null,
  );

  // Formulaire prérempli (ou sans erreur) à chaque ouverture seulement :
  // une liste des dossiers relue pendant la saisie ne l'efface pas.
  $effect(() => {
    const dialog = app.dialog;
    untrack(() => {
      const folder = app.targetFolder();
      if (dialog === "rename-folder" && folder !== null) renameFolder.reset(folder);
      if (dialog === "move-folder" && folder !== null && app.selectedAccountId !== null)
        moveFolder.reset(folder, mail.folders[app.selectedAccountId] ?? []);
      if (dialog === "delete-folder") deleteFolder.reset();
    });
  });

  /** Après un renommage, un déplacement ou une suppression : liste des
   * dossiers relue ; dossier affiché renommé ou déplacé (suivi sous son
   * nouveau nom) ou supprimé (retour à la boîte de réception du compte). */
  async function afterFolderChange(
    account: number,
    folder: FolderDto,
    renamedId: number | null,
  ): Promise<void> {
    const shown = mail.scope.kind === "mailbox" && mail.scope.id === folder.id;
    await mail.loadFolders(account);
    if (!shown) return;
    const inbox = (mail.folders[account] ?? []).find((f) => f.role === "inbox");
    if (renamedId !== null) selectScope({ kind: "mailbox", id: renamedId }, account);
    else if (inbox) selectScope({ kind: "mailbox", id: inbox.id }, account);
    else selectScope({ kind: "unified", role: "inbox" }, null);
  }

  /** Nom du contact dont la suppression est demandée, s'il est seul. */
  const deletionName = $derived.by(() => {
    const [id, ...others] = app.contactsToDelete;
    if (id === undefined || others.length > 0) return null;
    if (contacts.selected?.id === id) return contacts.selected.name;
    return contacts.items.find((c) => c.id === id)?.name ?? null;
  });

  /** Nom de la liste dont la suppression est demandée. */
  const listDeletionName = $derived.by(() => {
    const id = app.listToDelete;
    if (id === null) return "";
    if (lists.selected?.id === id) return lists.selected.fields.name;
    return lists.items.find((l) => l.id === id)?.name ?? "";
  });

  /** Nom du contact à ajouter à une liste, s'il est seul. */
  const addToListName = $derived.by(() => {
    const [id, ...others] = app.contactsToAdd;
    if (id === undefined || others.length > 0) return null;
    if (contacts.selected?.id === id) return contacts.selected.name;
    return contacts.items.find((c) => c.id === id)?.name ?? null;
  });

  const archiveFolders = $derived(
    app.selectedAccountId === null ? [] : (mail.folders[app.selectedAccountId] ?? []),
  );

  function selectScope(scope: ScopeDto, account: number | null): void {
    app.showMail();
    if (account !== null) app.selectedAccountId = account;
    search.close();
    void mail.show(scope);
  }

  /** Touche reçue de l'agent d'un message (focus dans le corps). */
  function onFrameKey(shortcut: Shortcut): void {
    if (app.dialog !== null) return;
    const command = registry.resolve(shortcut, app.context(), false);
    if (command !== undefined && relayFrameCommand(command.id))
      void registry.execute(command.id, app.context());
  }

  function isEditable(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;
    return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
  }

  const drag = new DragMove();

  /** Cible du panneau de gauche sous le pointeur : un dossier, ou la ligne
   * d'un compte (premier niveau). */
  function dropTargetAt(x: number, y: number): DropTarget | null {
    const element = document
      .elementFromPoint(x, y)
      ?.closest<HTMLElement>("[data-drop-folder], [data-drop-account]");
    if (!element) return null;
    const account = Number(element.dataset.dropAccount);
    if (element.dataset.dropFolder === undefined) return { account, folder: null };
    const id = Number(element.dataset.dropFolder);
    const folder = (mail.folders[account] ?? []).find((f) => f.id === id);
    return folder === undefined ? null : { account, folder };
  }

  /** Dossier déposé sur un autre (ou au premier niveau) : même
   * déplacement que « Déplacer le dossier… ». */
  async function dropFolder(
    account: number,
    folder: FolderDto,
    parent: FolderDto | null,
  ): Promise<void> {
    try {
      const id = await api.moveFolder(account, folder.id, parent?.id ?? null);
      mail.notice = {
        text: m.action_folder_moved({ folder: folderLabel(folder) }),
        undoable: false,
      };
      await afterFolderChange(account, folder, id);
    } catch (e) {
      mail.notice = {
        text: await folderFailureText(e, account, reloadFolders),
        undoable: false,
      };
    }
  }

  function onPointerMove(e: PointerEvent): void {
    if (!drag.owns(e.pointerId)) return;
    drag.move(e.clientX, e.clientY, dropTargetAt(e.clientX, e.clientY));
  }

  function onPointerCancel(e: PointerEvent): void {
    if (drag.owns(e.pointerId)) drag.cancel();
  }

  function onPointerUp(e: PointerEvent): void {
    if (!drag.owns(e.pointerId)) return;
    const drop = drag.release();
    if (drop === null) return;
    if (drop.payload.kind === "folder") {
      void dropFolder(drop.account, drop.payload.folder, drop.folder);
      return;
    }
    if (drop.folder === null) return;
    const scope = drop.payload.scope;
    void mail.act(
      { kind: "move", mailbox: drop.folder.id },
      m.action_moved({ folder: folderLabel(drop.folder) }),
      drop.payload.ids,
      scope === undefined ? undefined : { account: drop.payload.account, scope },
    );
  }

  /** Titre de la conversation ouverte saisi : début possible d'un glisser. */
  function grabOpened(e: PointerEvent): void {
    const opened = mail.opened;
    const scope = mail.openedScope();
    if (opened === null || scope === null || e.button !== 0) return;
    drag.grab(openedPayload(opened, scope), e);
  }

  // Pendant un glisser, une fois le pointeur éloigné du point de saisie, le
  // panneau de gauche défile quand le pointeur approche de son bord : les
  // dossiers hors de vue restent accessibles.
  $effect(() => {
    if (drag.payload === null || !drag.autoScroll) return;
    const pane = document.querySelector<HTMLElement>("[data-drop-scroll]");
    if (pane === null) return;
    let frame = requestAnimationFrame(function tick() {
      const step = edgeScroll(drag.x, drag.y, pane.getBoundingClientRect());
      const before = pane.scrollTop;
      if (step !== 0) pane.scrollTop += step;
      // La cible a changé sous le pointeur immobile.
      if (pane.scrollTop !== before) drag.move(drag.x, drag.y, dropTargetAt(drag.x, drag.y));
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  });

  // Doigt posé sur un élément saisissable : si le glisser commence, la page
  // ne défile plus (le défilement natif l'annulerait). Écouteur non passif,
  // posé seulement pendant cet appui, pour ne pas ralentir les autres
  // défilements au doigt.
  $effect(() => {
    if (!drag.touching) return;
    const hold = (e: TouchEvent): void => {
      if (drag.payload !== null && e.cancelable) e.preventDefault();
    };
    window.addEventListener("touchmove", hold, { passive: false });
    return () => window.removeEventListener("touchmove", hold);
  });

  /** Menu contextuel (clic droit, appui long) : retenu pendant un glisser
   * engagé, sinon il l'emporte et l'appui en cours est abandonné. */
  function onContextMenu(e: MouseEvent): void {
    if (drag.contextMenu()) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  // Pendant un glisser : ni sélection de texte, curseur « saisi ».
  $effect(() => {
    document.body.classList.toggle("dragging", drag.payload !== null);
  });

  function onKeyDown(e: KeyboardEvent): void {
    if (drag.payload !== null && e.key === "Escape") {
      e.preventDefault();
      drag.cancel();
      return;
    }
    // Les fenêtres modales gèrent leur propre clavier.
    if (app.dialog !== null || e.isComposing || e.defaultPrevented) return;
    const command = registry.resolve(shortcutFromEvent(e), app.context(), isEditable(e.target));
    if (command !== undefined) {
      e.preventDefault();
      void registry.execute(command.id, app.context());
    }
  }
</script>

<svelte:window
  onkeydown={onKeyDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerCancel}
  oncontextmenucapture={onContextMenu}
/>

{#if drag.payload !== null}
  <div
    class="drag-ghost"
    aria-hidden="true"
    style:left="{drag.x + 14}px"
    style:top="{drag.y + 14}px"
  >
    {drag.payload.kind === "folder"
      ? drag.payload.label
      : drag.payload.label === null
        ? m.drag_conversations({ count: drag.payload.ids.length })
        : drag.payload.label || m.common_no_subject()}
  </div>
{/if}

<ThreePaneLayout>
  {#snippet sidebar()}
    <Sidebar
      accounts={app.accounts}
      folders={mail.folders}
      offline={mail.offline}
      syncing={mail.syncing}
      reauth={mail.reauth}
      appPassword={mail.appPassword}
      scope={mail.scope}
      selectedId={app.selectedAccountId}
      onScope={selectScope}
      onAdd={() => app.open("add-account")}
      {registry}
      context={app.context()}
      onAccount={(id) => {
        app.selectedAccountId = id;
        return app.context();
      }}
      {drag}
      contactsActive={app.view === "contacts"}
      onFolder={(account, folder, target) => {
        if (!target) {
          return { ...app.context(), selectedAccount: app.accountById(account), folder };
        }
        app.selectedAccountId = account;
        app.folderTarget = folder;
        return app.context();
      }}
    />
  {/snippet}
  {#snippet list()}
    {#if app.view === "contacts"}
      <ContactList
        {contacts}
        {lists}
        {registry}
        context={app.context()}
        onPoint={(id) => (app.contactTarget = id)}
        onPointList={(id) => (app.listTarget = id)}
      />
    {:else if search.active}
      <SearchPanel {search} onOpen={() => void openSelectedHit()} {drag} />
    {:else if app.accounts.length > 0}
      <ConversationList
        {mail}
        title={listTitle}
        {registry}
        context={app.context()}
        onSearch={() => search.open()}
        {drag}
        person={mail.targetSender()}
        personContext={(p) => app.contextForPerson(p)}
        onPerson={(id, p) => void runForPerson(id, p)}
        labelsOf={(account) => labelsStore.of(account)}
        onLabel={searchLabel}
      />
    {/if}
  {/snippet}
  {#snippet reader()}
    {#if app.view === "contacts" && lists.shown}
      {#if lists.selected !== null}
        <ContactListCard list={lists.selected} {registry} context={app.context()} />
      {:else}
        <div class="placeholder">
          <UsersIcon size={40} aria-hidden="true" />
          <p>{m.lists_pick()}</p>
        </div>
      {/if}
    {:else if app.view === "contacts"}
      {#if contacts.selected !== null}
        <ContactCard
          contact={contacts.selected}
          {registry}
          context={app.context()}
          openWebsite={(url) => api.openExternal(url)}
        />
      {:else}
        <div class="placeholder">
          <BookUserIcon size={40} aria-hidden="true" />
          <p>{m.contacts_pick()}</p>
        </div>
      {/if}
    {:else if mail.opened !== null}
      <ConversationView
        conversation={mail.opened}
        messages={mail.messages}
        {api}
        onBack={() => mail.back()}
        backTitle={describeCommand(
          registry,
          "conversation.back",
          app.context(),
          m.conversation_back(),
        )?.tooltip ?? m.conversation_back()}
        onKey={onFrameKey}
        onGrab={grabOpened}
      >
        {#snippet person(p, strong)}
          <PersonMenu
            person={p}
            {registry}
            context={app.contextForPerson(p)}
            run={(id) => void runForPerson(id, p)}
            {strong}
          />
        {/snippet}
        {#snippet labels()}
          {@const opened = mail.opened}
          {@const chips =
            opened === null
              ? []
              : selectionChips(
                  mail.messages.map((x) => x.labels),
                  labelsStore.of(opened.account),
                )}
          {#if chips.length > 0}
            <ul class="conversation-labels" aria-label={m.label_list()}>
              {#each chips as label (label.keyword)}
                <li><LabelChip {label} onSearch={searchLabel} /></li>
              {/each}
            </ul>
          {/if}
        {/snippet}
        {#snippet toolbar()}
          {@const actions = readerActions(mail.opened, app.context().person)}
          <ActionBar
            primary={actions.primary}
            more={actions.more}
            {registry}
            context={app.context()}
            label={m.conversation_toolbar()}
            rows={actions.rows}
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
<ShortcutHelp
  open={app.dialog === "help"}
  {registry}
  context={app.context()}
  onClose={() => app.close()}
  onPalette={() => app.open("palette")}
/>
<AddAccountDialog
  open={app.dialog === "add-account"}
  {api}
  onClose={() => app.close()}
  onAdded={(account, notice) => {
    app.close();
    if (notice !== null) mail.notice = { text: notice, undoable: false };
    app.selectedAccountId = account.id;
    void app.refreshAccounts().then(() => mail.reload());
  }}
/>
<FolderPicker
  open={app.dialog === "move"}
  title={m.picker_move_title()}
  folders={moveFolders}
  onClose={() => app.close()}
  onCreate={(name) => {
    // Le dossier est créé dans le compte des conversations à déplacer.
    if (moveAccount() === null) {
      app.close();
      mail.notice = { text: m.action_one_account(), undoable: false };
      return;
    }
    // Conversations visées relevées maintenant : une liste relue pendant
    // la création ne change pas ce qui sera déplacé.
    app.openNewFolder({ name }, mail.captureTargets());
  }}
  onChoose={(folder) => {
    app.close();
    void mail.act(
      { kind: "move", mailbox: folder.id },
      m.action_moved({ folder: folderLabel(folder) }),
    );
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
          text: m.action_archive_folder_set({ folder: folderLabel(folder) }),
          undoable: false,
        }),
      (e: unknown) => (mail.notice = { text: errorText(e), undoable: false }),
    );
  }}
/>
<NewFolderDialog
  open={app.dialog === "new-folder"}
  account={newFolderAccount === null ? null : app.accountById(newFolderAccount)}
  folders={newFolderAccount === null ? [] : (mail.folders[newFolderAccount] ?? [])}
  form={newFolder}
  moving={app.moveAfterCreate !== null}
  onClose={() => app.close()}
  onCreated={(created) => void app.folderCreated(created)}
/>
<RenameFolderDialog
  open={app.dialog === "rename-folder"}
  account={app.selectedAccount}
  folder={editedFolder}
  form={renameFolder}
  onClose={() => app.close()}
  onRenamed={(renamed) => {
    const account = app.selectedAccountId;
    const folder = editedFolder;
    app.close();
    mail.notice = { text: m.action_folder_renamed({ folder: renamed.name }), undoable: false };
    if (account !== null && folder !== null) void afterFolderChange(account, folder, renamed.id);
  }}
/>
<MoveFolderDialog
  open={app.dialog === "move-folder"}
  account={app.selectedAccount}
  folder={editedFolder}
  folders={archiveFolders}
  form={moveFolder}
  onClose={() => app.close()}
  onMoved={(moved) => {
    const account = app.selectedAccountId;
    const folder = editedFolder;
    app.close();
    mail.notice = { text: m.action_folder_moved({ folder: moved.name }), undoable: false };
    if (account !== null && folder !== null) void afterFolderChange(account, folder, moved.id);
  }}
/>
<DeleteFolderDialog
  open={app.dialog === "delete-folder"}
  account={app.selectedAccount}
  folder={editedFolder}
  form={deleteFolder}
  onClose={() => app.close()}
  onDeleted={(folder) => {
    const account = app.selectedAccountId;
    app.close();
    mail.notice = {
      text: m.action_folder_deleted({ folder: folderLabel(folder) }),
      undoable: false,
    };
    if (account !== null) void afterFolderChange(account, folder, null);
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
<LabelDialog open={app.dialog === "label"} state={labelMenu} onClose={() => app.close()} />
<SettingsDialog
  open={app.dialog === "settings"}
  settings={settings.current}
  plainText={app.plainText}
  updatesSupported={updates.supported}
  error={settings.error}
  onClose={() => app.close()}
  onSave={async (next, plain) => {
    const saved = await settings.save(next);
    if (!saved) return;
    if (plain !== app.plainText) await api.setPreferPlainText(plain).catch(() => undefined);
    app.close();
  }}
>
  {#snippet extra()}
    <LabelSettings accounts={app.accounts} state={labelSettings} labels={labelsStore} />
  {/snippet}
</SettingsDialog>
<UpdateBanner
  version={updates.available}
  installing={updates.installing}
  error={updates.error}
  onInstall={() => void updates.install()}
  onDismiss={() => updates.dismiss()}
/>
<UpdateQuestion
  open={settings.askUpdate && updates.supported && app.dialog === null}
  onAnswer={(yes) => void settings.answerUpdate(yes)}
  onDismiss={() => settings.dismissUpdate()}
/>
<ContactDialog
  open={app.dialog === "contact"}
  form={contactForm}
  onClose={() => app.close()}
  onSaved={(contact) => {
    const created = contactForm.id === null;
    app.close();
    mail.notice = {
      text: created
        ? m.contacts_notice_created({ name: contact.name })
        : m.contacts_notice_saved({ name: contact.name }),
      undoable: false,
    };
    void contacts.saved(contact);
    // Nom du carnet dans la liste et le message ouvert.
    if (app.view === "mail") void mail.reload();
  }}
/>
<DeleteContactsDialog
  open={app.dialog === "delete-contacts"}
  ids={app.contactsToDelete}
  name={deletionName}
  form={contactDeletion}
  onClose={() => app.close()}
  onDeleted={(ids, count) => {
    app.close();
    mail.notice = { text: m.contacts_notice_deleted({ count }), undoable: false };
    void contacts.removed(ids);
  }}
/>
<ContactDuplicatesDialog
  open={app.dialog === "contact-duplicates"}
  form={contactDuplicates}
  onMerged={() => void contacts.load()}
  onClose={() => {
    const merged = contactDuplicates.merged;
    contactDuplicates.close();
    app.close();
    if (merged > 0) {
      mail.notice = { text: m.contacts_duplicates_notice({ count: merged }), undoable: false };
      if (app.view === "mail") void mail.reload();
    }
  }}
/>
<ContactExportDialog
  open={app.dialog === "contact-export"}
  form={contactExport}
  onClose={() => {
    contactExport.close();
    app.close();
  }}
  onExported={(outcome) => {
    app.close();
    mail.notice = {
      text: m.contacts_export_notice({ count: outcome.exported, name: outcome.file_name }),
      undoable: false,
    };
  }}
/>
<CardDavDialog
  open={app.dialog === "carddav"}
  form={carddav}
  onClose={() => {
    carddav.close();
    app.close();
  }}
  onSynced={(report) => {
    mail.notice = {
      text: m.carddav_notice({ pulled: report.pulled, pushed: report.pushed }),
      undoable: false,
    };
    if (report.pulled > 0) {
      void contacts.load();
      if (app.view === "mail") void mail.reload();
    }
  }}
/>
<ContactImportDialog
  open={app.dialog === "contact-import"}
  form={contactImport}
  onClose={() => {
    contactImport.close();
    app.close();
  }}
  onImported={(outcome) => {
    app.close();
    mail.notice = {
      text:
        outcome.lists_created + outcome.lists_completed > 0
          ? m.contacts_import_notice_lists({
              created: outcome.created,
              completed: outcome.completed,
              lists: outcome.lists_created,
              listsCompleted: outcome.lists_completed,
            })
          : m.contacts_import_notice({ created: outcome.created, completed: outcome.completed }),
      undoable: false,
    };
    void contacts.load();
    if (app.view === "mail") void mail.reload();
  }}
/>
<ContactListDialog
  open={app.dialog === "contact-list"}
  form={listForm}
  onClose={() => app.close()}
  onSaved={(list) => {
    const created = listForm.id === null;
    app.close();
    mail.notice = {
      text: created
        ? m.lists_notice_created({ name: list.fields.name })
        : m.lists_notice_saved({ name: list.fields.name }),
      undoable: false,
    };
    void lists.saved(list);
  }}
/>
<AddToListDialog
  open={app.dialog === "add-to-list"}
  ids={app.contactsToAdd}
  name={addToListName}
  form={addToList}
  onClose={() => app.close()}
  onAdded={(list) => {
    app.close();
    mail.notice = { text: m.lists_notice_added({ name: list.fields.name }), undoable: false };
    if (lists.shown) void lists.load();
  }}
/>
<DeleteContactListDialog
  open={app.dialog === "delete-contact-list"}
  id={app.listToDelete}
  name={listDeletionName}
  form={listDeletion}
  onClose={() => app.close()}
  onDeleted={(id) => {
    const name = listDeletionName;
    app.close();
    mail.notice = { text: m.lists_notice_deleted({ name }), undoable: false };
    void lists.removed(id);
  }}
/>
<ChangePasswordDialog
  open={app.dialog === "change-password"}
  {api}
  account={app.selectedAccount}
  appPasswordHelp={app.selectedAccount !== null &&
    mail.appPassword[app.selectedAccount.id] === true}
  onClose={() => app.close()}
  onChanged={(account) => {
    app.close();
    mail.passwordChanged(account.id);
  }}
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
  /* Étiquettes de la conversation ouverte, sous le sujet. */
  .conversation-labels {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin: 0;
    padding: 0 var(--space-4) var(--space-2);
    list-style: none;
  }

  /* Étiquette qui suit le pointeur pendant un glisser. */
  .drag-ghost {
    position: fixed;
    z-index: 1000;
    max-width: 280px;
    padding: var(--space-1) var(--space-2);
    overflow: hidden;
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
    background: var(--color-bg);
    box-shadow: var(--shadow-dialog);
    color: var(--color-text);
    font-size: var(--font-size-sm);
    white-space: nowrap;
    text-overflow: ellipsis;
    pointer-events: none;
  }

  :global(body.dragging) {
    cursor: grabbing;
    user-select: none;
  }

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
