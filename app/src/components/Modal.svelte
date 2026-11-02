<script lang="ts">
  // Fenêtre modale accessible (bits-ui Dialog) : focus piégé, Échap, titre.
  import { Dialog } from "bits-ui";
  import type { Snippet } from "svelte";

  interface Props {
    open: boolean;
    title: string;
    description?: string;
    width?: string;
    onClose: () => void;
    /** Empêche la fermeture (opération en cours). */
    locked?: boolean;
    children: Snippet;
  }

  let {
    open,
    title,
    description,
    width = "480px",
    onClose,
    locked = false,
    children,
  }: Props = $props();
</script>

<Dialog.Root
  {open}
  onOpenChange={(o) => {
    if (!o && !locked) onClose();
  }}
>
  <Dialog.Portal>
    <Dialog.Overlay class="modal-overlay" />
    <Dialog.Content
      class="modal-content"
      style="--modal-width: {width}"
      escapeKeydownBehavior={locked ? "ignore" : "close"}
      interactOutsideBehavior={locked ? "ignore" : "close"}
    >
      <Dialog.Title class="modal-title">{title}</Dialog.Title>
      {#if description}
        <Dialog.Description class="modal-description">{description}</Dialog.Description>
      {/if}
      {@render children()}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  :global(.modal-overlay) {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: var(--color-overlay);
  }

  :global(.modal-content) {
    position: fixed;
    top: 12vh;
    left: 50%;
    z-index: 51;
    width: min(var(--modal-width), calc(100vw - 2 * var(--space-4)));
    max-height: 76vh;
    overflow: auto;
    padding: var(--space-5);
    transform: translateX(-50%);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-dialog);
  }

  :global(.modal-title) {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-bold);
  }

  :global(.modal-description) {
    margin: 0 0 var(--space-4);
    color: var(--color-text-muted);
  }
</style>
