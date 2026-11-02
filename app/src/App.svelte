<script lang="ts">
  import { onMount } from "svelte";
  import InboxIcon from "@lucide/svelte/icons/inbox";
  import ThreePaneLayout from "./components/ThreePaneLayout.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import CommandPalette from "./components/CommandPalette.svelte";
  import ShortcutHelp from "./components/ShortcutHelp.svelte";
  import AddAccountDialog from "./components/AddAccountDialog.svelte";
  import RemoveAccountDialog from "./components/RemoveAccountDialog.svelte";
  import { app, registerAppCommands, registry } from "./lib/app.svelte";
  import { shortcutFromEvent } from "./lib/commands/shortcut";
  import { api } from "./lib/ipc/api";

  onMount(() => {
    const unregister = registerAppCommands();
    void app.refreshAccounts();
    return unregister;
  });

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
      selectedId={app.selectedAccountId}
      onSelect={(id) => (app.selectedAccountId = id)}
      onAdd={() => app.open("add-account")}
      onRemove={(id) => {
        app.selectedAccountId = id;
        app.open("remove-account");
      }}
    />
  {/snippet}
  {#snippet list()}
    <div class="placeholder">
      {#if app.selectedAccount}
        <p>Les messages de {app.selectedAccount.address} apparaîtront ici.</p>
      {/if}
    </div>
  {/snippet}
  {#snippet reader()}
    <div class="placeholder welcome">
      <InboxIcon size={40} aria-hidden="true" />
      <h1>Norelly</h1>
      {#if app.loadError}
        <p role="alert">Impossible de lire les comptes : {app.loadError}</p>
      {:else if app.accounts.length === 0}
        <p>Aucun compte pour l'instant.</p>
        <button class="btn btn-primary" type="button" onclick={() => app.open("add-account")}
          >Ajouter un compte</button
        >
      {/if}
      <p class="hint">
        Palette de commandes : <kbd>Ctrl</kbd> <kbd>K</kbd> · Raccourcis : <kbd>?</kbd>
      </p>
    </div>
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
    void app.refreshAccounts();
  }}
/>
<RemoveAccountDialog
  open={app.dialog === "remove-account"}
  account={app.selectedAccount}
  onClose={() => app.close()}
  onConfirm={async (account) => {
    await api.removeAccount(account.id);
    app.close();
    app.selectedAccountId = null;
    await app.refreshAccounts();
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
