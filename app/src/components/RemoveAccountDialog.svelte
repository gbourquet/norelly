<script lang="ts">
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
  title="Supprimer le compte ?"
  onClose={() => {
    error = null;
    onClose();
  }}
  locked={busy}
>
  {#if account}
    <p>
      Le compte <strong>{account.address}</strong> sera retiré de Norelly : son mot de passe est effacé
      du trousseau et ses données locales sont supprimées. Les messages restent sur le serveur.
    </p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  <div class="actions">
    <button class="btn" type="button" onclick={onClose} disabled={busy}>Annuler</button>
    <button
      class="btn btn-danger"
      type="button"
      onclick={() => void confirm()}
      disabled={busy || account === null}
    >
      Supprimer le compte
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
