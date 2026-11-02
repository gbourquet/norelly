<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Confirmation de suppression d'un compte.
  import Modal from "./Modal.svelte";
  import type { AccountDto } from "../lib/ipc/api";

  interface Props {
    open: boolean;
    account: AccountDto | null;
    onConfirm: (account: AccountDto) => Promise<void>;
    onClose: () => void;
  }

  let { open, account, onConfirm, onClose }: Props = $props();

  let busy = $state(false);
  let error = $state<string | null>(null);

  async function confirm(): Promise<void> {
    if (account === null) return;
    busy = true;
    error = null;
    try {
      await onConfirm(account);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal
  {open}
  title={m.remove_title()}
  onClose={() => {
    error = null;
    onClose();
  }}
  locked={busy}
>
  {#if account}
    <p>
      {m.remove_explanation_before()} <strong>{account.address}</strong>
      {m.remove_explanation_after()}
    </p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  <div class="actions">
    <button class="btn" type="button" onclick={onClose} disabled={busy}>{m.common_cancel()}</button>
    <button
      class="btn btn-danger"
      type="button"
      onclick={() => void confirm()}
      disabled={busy || account === null}
    >
      {m.remove_confirm()}
    </button>
  </div>
</Modal>

<style>
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-4);
  }

  .error {
    color: var(--color-danger);
  }
</style>
