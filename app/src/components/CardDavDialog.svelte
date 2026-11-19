<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Synchronisation du carnet avec un serveur CardDAV : adresse du
  // serveur, identifiant et mot de passe d'application, choix du carnet
  // parmi ceux que Rust a trouvés ; une fois relié, état de la dernière
  // synchronisation, « Synchroniser maintenant » et « Déconnecter ».
  import Modal from "./Modal.svelte";
  import type { CardDavState } from "../lib/carddav.svelte";
  import type { CardDavFailureDto, CardDavReportDto } from "../lib/ipc/api";
  import { longDate } from "../lib/format";

  interface Props {
    open: boolean;
    form: CardDavState;
    onSynced: (report: CardDavReportDto) => void;
    onClose: () => void;
  }

  let { open, form, onSynced, onClose }: Props = $props();

  function failureText(failure: CardDavFailureDto): string {
    switch (failure) {
      case "unauthorized":
        return m.carddav_failure_unauthorized();
      case "network":
        return m.carddav_failure_network();
      case "certificate":
        return m.carddav_failure_certificate();
      case "not_found":
        return m.carddav_failure_not_found();
      case "too_large":
        return m.carddav_failure_too_large();
      case "server":
        return m.carddav_failure_server();
      case "storage":
        return m.carddav_failure_storage();
      case "keyring":
        return m.carddav_failure_keyring();
    }
  }

  async function connect(): Promise<void> {
    const report = await form.connect();
    if (report !== null) onSynced(report);
  }

  async function sync(): Promise<void> {
    const report = await form.sync();
    if (report !== null) onSynced(report);
  }

  const account = $derived(form.status?.account ?? null);
  const last = $derived(form.status?.last_sync ?? null);
</script>

<Modal {open} title={m.carddav_title()} width="560px" {onClose} locked={form.busy}>
  {#if account !== null}
    <p class="linked">
      {m.carddav_linked({
        book: account.book_name || m.carddav_book_unnamed(),
        username: account.username,
        server: account.server,
      })}
    </p>
    <p class="status" role="status">
      {#if form.busy}
        {m.carddav_syncing()}
      {:else if last === null}
        {m.carddav_never()}
      {:else if last.report !== null}
        {m.carddav_last_ok({
          date: longDate(last.at),
          pulled: last.report.pulled,
          pushed: last.report.pushed,
        })}
        {#if last.report.skipped > 0}
          <span class="skipped">{m.carddav_last_skipped({ count: last.report.skipped })}</span>
        {/if}
      {:else if last.failure !== null}
        {m.carddav_last_failed({ date: longDate(last.at), reason: failureText(last.failure) })}
      {/if}
    </p>
    <p class="note">{m.carddav_conflicts_note()}</p>
    <p class="note">{m.carddav_disconnect_hint()}</p>
    {#if form.error}
      <p class="error" role="alert">{form.error}</p>
    {/if}
    <div class="actions">
      <button
        class="btn btn-danger"
        type="button"
        onclick={() => void form.disconnect()}
        disabled={form.busy}>{m.carddav_disconnect()}</button
      >
      <span class="spacer"></span>
      <button class="btn" type="button" onclick={onClose} disabled={form.busy}
        >{m.common_close()}</button
      >
      <button
        class="btn btn-primary"
        type="button"
        onclick={() => void sync()}
        disabled={form.busy}
      >
        {form.busy ? m.carddav_syncing() : m.carddav_sync_now()}
      </button>
    </div>
  {:else}
    <p class="intro">{m.carddav_intro()}</p>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        if (form.books === null) void form.search();
        else void connect();
      }}
    >
      <label class="field">
        <span>{m.carddav_server()}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="input"
          type="text"
          inputmode="url"
          autocomplete="url"
          spellcheck="false"
          bind:value={form.server}
          oninput={() => form.edited()}
          aria-describedby="carddav-server-hint"
          autofocus
          disabled={form.busy}
        />
        <span id="carddav-server-hint" class="hint">{m.carddav_server_hint()}</span>
      </label>
      <label class="field">
        <span>{m.carddav_username()}</span>
        <input
          class="input"
          type="text"
          autocomplete="username"
          spellcheck="false"
          bind:value={form.username}
          oninput={() => form.edited()}
          disabled={form.busy}
        />
      </label>
      <label class="field">
        <span>{m.carddav_password()}</span>
        <input
          class="input"
          type="password"
          autocomplete="current-password"
          bind:value={form.password}
          aria-describedby="carddav-password-hint"
          disabled={form.busy}
        />
        <span id="carddav-password-hint" class="hint">{m.carddav_password_hint()}</span>
      </label>
      {#if form.books !== null}
        <fieldset class="books">
          <legend>{m.carddav_books()}</legend>
          {#each form.books as book (book.url)}
            <label>
              <input
                type="radio"
                name="carddav-book"
                value={book.url}
                bind:group={form.book}
                disabled={form.busy}
              />
              <span class="text">
                <span class="name">{book.name || m.carddav_book_unnamed()}</span>
                <span class="hint">{book.url}</span>
              </span>
            </label>
          {/each}
        </fieldset>
      {/if}
      {#if form.error}
        <p class="error" role="alert">{form.error}</p>
      {/if}
      <div class="actions">
        <button class="btn" type="button" onclick={onClose} disabled={form.busy}
          >{m.common_cancel()}</button
        >
        {#if form.books === null}
          <button class="btn btn-primary" type="submit" disabled={!form.canSearch}>
            {form.busy ? m.carddav_searching() : m.carddav_search()}
          </button>
        {:else}
          <button
            class="btn btn-primary"
            type="submit"
            disabled={form.busy || form.book === null || form.password === ""}
          >
            {form.busy ? m.carddav_syncing() : m.carddav_connect()}
          </button>
        {/if}
      </div>
    </form>
  {/if}
</Modal>

<style>
  .intro,
  .linked,
  .status {
    margin: 0 0 var(--space-3);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .books {
    margin: 0;
    padding: var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .books legend {
    padding: 0 var(--space-1);
    font-weight: var(--font-weight-medium);
  }

  .books label {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    cursor: pointer;
    min-width: 0;
  }

  .books input {
    margin-top: 3px;
  }

  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .text .hint {
    overflow-wrap: anywhere;
  }

  .hint,
  .note,
  .skipped {
    color: var(--color-text-muted);
  }

  .hint {
    font-size: var(--font-size-sm);
  }

  .skipped {
    display: block;
  }

  .note {
    margin: 0 0 var(--space-2);
  }

  .linked {
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-4);
  }

  .spacer {
    flex: 1;
  }

  .error {
    color: var(--color-danger);
    margin: 0;
  }
</style>
