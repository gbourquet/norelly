<script lang="ts">
  import { m } from "./lib/paraglide/messages.js";
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
  import { app, mail, registerAppCommands, registry } from "./lib/app.svelte";
  import { shortcutFromEvent, type Shortcut } from "./lib/commands/shortcut";
  import { api, type ScopeDto } from "./lib/ipc/api";
  import { UNIFIED_INBOX } from "./lib/mail.svelte";

  onMount(() => {
    const unregister = registerAppCommands();
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void (async () => {
      await app.refreshAccounts();
      await mail.show(UNIFIED_INBOX);
      try {
        const off = await api.onSyncEvent((event) => void mail.onSyncEvent(event));
        if (disposed) off();
        else unlisten = off;
      } catch {
        // Hors de Tauri (tests) : pas d'événements.
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
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

  function selectScope(scope: ScopeDto, account: number | null): void {
    if (account !== null) app.selectedAccountId = account;
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
    {#if app.accounts.length > 0}
      <ConversationList {mail} title={listTitle} />
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
      />
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
