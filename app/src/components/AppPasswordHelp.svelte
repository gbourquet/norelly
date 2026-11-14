<script lang="ts">
  // Étapes de création d'un mot de passe d'application Google. Affichées
  // seulement quand Rust l'a décidé (compte entièrement chez Google).
  import { m } from "../lib/paraglide/messages.js";
  import KeyRoundIcon from "@lucide/svelte/icons/key-round";
  import ExternalLinkIcon from "@lucide/svelte/icons/external-link";

  interface Props {
    /** Dernière étape : où coller le code. */
    paste: string;
    onOpen: () => void;
  }

  let { paste, onOpen }: Props = $props();
  const id = $props.id();
</script>

<section class="app-password" aria-labelledby="{id}-title">
  <h3 id="{id}-title">
    <KeyRoundIcon size={18} aria-hidden="true" />
    {m.add_app_password_title()}
  </h3>
  <ol>
    <li>{m.add_app_password_step_2fa()}</li>
    <li>
      {m.add_app_password_step_create()}
      <button class="btn" type="button" onclick={onOpen}>
        <ExternalLinkIcon size={14} aria-hidden="true" />
        {m.add_app_password_open()}
      </button>
    </li>
    <li>{paste}</li>
  </ol>
  <p class="hint">{m.add_app_password_unavailable()}</p>
</section>

<style>
  .app-password {
    margin: 0 0 var(--space-3);
    padding: var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
    background: var(--color-surface);
  }

  .app-password h3 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-md);
  }

  .app-password ol {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding-left: var(--space-5);
  }

  .app-password li .btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-top: var(--space-1);
  }

  .hint {
    margin: var(--space-1) 0 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }
</style>
