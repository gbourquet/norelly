<script lang="ts">
  // Premier lancement : faut-il rechercher les mises à jour ? « Oui » est
  // présélectionné (premier bouton, qui reçoit le focus).
  import { m } from "../lib/paraglide/messages.js";
  import Modal from "./Modal.svelte";

  interface Props {
    open: boolean;
    onAnswer: (yes: boolean) => void;
  }

  let { open, onAnswer }: Props = $props();

  let yes: HTMLButtonElement | null = $state(null);

  // « Oui » présélectionné : il reçoit le focus à l'ouverture (après celui
  // que pose la fenêtre modale).
  $effect(() => {
    if (open && yes !== null) {
      const button = yes;
      const timer = setTimeout(() => button.focus(), 0);
      return () => clearTimeout(timer);
    }
  });
</script>

<Modal {open} title={m.update_question_title()} onClose={() => onAnswer(true)}>
  <p>{m.update_question()}</p>
  <div class="actions">
    <button bind:this={yes} class="btn btn-primary" type="button" onclick={() => onAnswer(true)}
      >{m.update_yes()}</button
    >
    <button class="btn" type="button" onclick={() => onAnswer(false)}>{m.update_no()}</button>
  </div>
</Modal>

<style>
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-4);
  }
</style>
