<script lang="ts">
  // Annonce d'une mise à jour trouvée et vérifiée par Rust (installateur
  // NSIS seulement) ; l'installation ferme l'application.
  import { m } from "../lib/paraglide/messages.js";

  interface Props {
    version: string | null;
    installing: boolean;
    error: string | null;
    onInstall: () => void;
    onDismiss: () => void;
  }

  let { version, installing, error, onInstall, onDismiss }: Props = $props();
</script>

<div class="update-region" role="status" aria-live="polite">
  {#if version !== null}
    <div class="update">
      <span>{m.update_available({ version })}</span>
      {#if error}<span class="error">{error}</span>{/if}
      <button class="btn btn-primary" type="button" disabled={installing} onclick={onInstall}>
        {installing ? m.update_installing() : m.update_install()}
      </button>
      <button class="btn btn-ghost" type="button" disabled={installing} onclick={onDismiss}>
        {m.update_later()}
      </button>
    </div>
  {/if}
</div>

<style>
  .update-region {
    position: fixed;
    top: var(--space-4);
    right: var(--space-4);
    z-index: 40;
  }

  .update {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-dialog);
  }

  .error {
    color: var(--color-danger);
  }
</style>
