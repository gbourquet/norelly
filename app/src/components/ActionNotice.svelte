<script lang="ts">
  // Information après une action (« Archivé. Annuler »), annoncée aux
  // lecteurs d'écran ; disparaît d'elle-même.
  import { UNDO_WINDOW_MS, type Notice } from "../lib/mail.svelte";
  import { m } from "../lib/paraglide/messages.js";

  interface Props {
    notice: Notice | null;
    canUndo: boolean;
    onUndo: () => void;
    onDismiss: () => void;
  }

  let { notice, canUndo, onUndo, onDismiss }: Props = $props();

  $effect(() => {
    if (notice === null) return;
    const timer = setTimeout(onDismiss, UNDO_WINDOW_MS);
    return () => clearTimeout(timer);
  });
</script>

<div class="notice-region" role="status" aria-live="polite">
  {#if notice}
    <div class="notice">
      <span>{notice.text}</span>
      {#if notice.undoable && canUndo}
        <button class="btn btn-ghost" type="button" onclick={onUndo}
          >{m.action_undo()} <kbd>Z</kbd></button
        >
      {/if}
    </div>
  {/if}
</div>

<style>
  .notice-region {
    position: fixed;
    bottom: var(--space-4);
    left: 50%;
    z-index: 40;
    transform: translateX(-50%);
  }

  .notice {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-dialog);
  }
</style>
