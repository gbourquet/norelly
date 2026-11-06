<script lang="ts">
  // Fenêtre modale accessible (bits-ui Dialog) : focus piégé, Échap, titre.
  import { Dialog } from "bits-ui";
  import XIcon from "@lucide/svelte/icons/x";
  import { m } from "../lib/paraglide/messages.js";
  import type { Snippet } from "svelte";

  interface Props {
    open: boolean;
    title: string;
    description?: string;
    width?: string;
    onClose: () => void;
    /** Empêche la fermeture (opération en cours). */
    locked?: boolean;
    /** Focus sur la fenêtre elle-même à l'ouverture, plutôt que sur son
     * premier élément (contenu à lire, premier bouton tout en bas). */
    focusContent?: boolean;
    children: Snippet;
  }

  let {
    open,
    title,
    description,
    width = "480px",
    onClose,
    locked = false,
    focusContent = false,
    children,
  }: Props = $props();

  let content: HTMLElement | null = $state(null);

  function onOpenAutoFocus(e: Event): void {
    if (!focusContent) return;
    e.preventDefault();
    content?.focus();
  }
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
      bind:ref={content}
      {onOpenAutoFocus}
    >
      <Dialog.Title class="modal-title">{title}</Dialog.Title>
      {#if description}
        <Dialog.Description class="modal-description">{description}</Dialog.Description>
      {/if}
      {@render children()}
      <!-- Fermeture à la souris (Échap et clic à l'extérieur aussi) : en
           haut à droite, mais dernier dans l'ordre de tabulation pour que
           le premier champ reçoive le focus à l'ouverture. -->
      <Dialog.Close
        class="btn btn-ghost modal-close"
        disabled={locked}
        aria-label={m.common_close()}
        title={m.common_close()}
      >
        <XIcon size={16} aria-hidden="true" />
      </Dialog.Close>
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

  :global(.modal-close) {
    position: absolute;
    top: var(--space-4);
    right: var(--space-4);
    padding: var(--space-1);
    min-height: 0;
  }

  :global(.modal-title) {
    margin: 0 0 var(--space-2);
    padding-right: var(--space-8);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-bold);
  }

  :global(.modal-description) {
    margin: 0 0 var(--space-4);
    color: var(--color-text-muted);
  }
</style>
