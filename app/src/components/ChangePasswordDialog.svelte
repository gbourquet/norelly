<script lang="ts">
  // Changement du mot de passe d'un compte (voir lib/change-password.svelte.ts).
  import { untrack } from "svelte";
  import LoaderIcon from "@lucide/svelte/icons/loader-circle";
  import AppPasswordHelp from "./AppPasswordHelp.svelte";
  import Modal from "./Modal.svelte";
  import { ChangePasswordForm, type ChangePasswordApi } from "../lib/change-password.svelte";
  import { m } from "../lib/paraglide/messages.js";
  import type { AccountDto } from "../lib/ipc/api";

  interface Props {
    open: boolean;
    api: ChangePasswordApi;
    account: AccountDto | null;
    /** Google refuse déjà le mot de passe d'application de ce compte. */
    appPasswordHelp: boolean;
    onClose: () => void;
    onChanged: (account: AccountDto) => void;
  }

  let { open, api, account, appPasswordHelp, onClose, onChanged }: Props = $props();

  const accountId = $derived(account?.id ?? null);
  /** Formulaire neuf à chaque ouverture, pour le compte visé ; l'aide
      reçue ensuite ne l'efface pas (saisie en cours gardée). */
  const form = $derived(
    open && accountId !== null
      ? new ChangePasswordForm(
          api,
          accountId,
          untrack(() => appPasswordHelp),
        )
      : null,
  );

  async function submit(): Promise<void> {
    if (form === null || account === null) return;
    const target = account;
    if (await form.submit()) onChanged(target);
  }
</script>

<Modal {open} title={m.password_title()} {onClose} locked={form?.busy ?? false} width="520px">
  {#if form && account}
    {#if form.message}
      <p class="message" role="alert">{form.message}</p>
    {/if}
    {#if form.appPasswordHelp}
      <AppPasswordHelp
        paste={m.password_app_password_step_paste()}
        onOpen={() => void form.openAppPasswords()}
      />
    {/if}
    <form
      onsubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <p class="explanation">{m.password_explanation({ address: account.address })}</p>
      <label for="new-password">{m.password_label()}</label>
      <input
        id="new-password"
        class="input"
        type="password"
        bind:value={form.password}
        autocomplete="new-password"
        disabled={form.busy}
        required
      />
      {#if form.busy}
        <p class="testing" role="status">
          <LoaderIcon size={14} class="spin" aria-hidden="true" />
          {m.password_testing()}
        </p>
      {/if}
      <div class="actions">
        <button class="btn" type="button" onclick={onClose} disabled={form.busy}>
          {m.common_cancel()}
        </button>
        <button class="btn btn-primary" type="submit" disabled={form.busy}>
          {m.password_save()}
        </button>
      </div>
    </form>
  {/if}
</Modal>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .explanation {
    margin: 0 0 var(--space-2);
  }

  label {
    font-weight: var(--font-weight-medium);
  }

  .testing {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-2) 0 0;
    color: var(--color-text-muted);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }

  .message {
    margin: 0 0 var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius);
    background: var(--color-warning-bg);
    color: var(--color-warning);
  }
</style>
