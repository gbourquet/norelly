<script lang="ts">
  // Paramètres : langue, mises à jour, notifications, affichage,
  // rapatriement. Les valeurs sont revalidées par Rust à l'enregistrement.
  import { m } from "../lib/paraglide/messages.js";
  import Modal from "./Modal.svelte";
  import type { SettingsDto } from "../lib/ipc/api";

  interface Props {
    open: boolean;
    settings: SettingsDto | null;
    plainText: boolean;
    error: string | null;
    onSave: (next: SettingsDto, plainText: boolean) => void;
    onClose: () => void;
  }

  let { open, settings, plainText, error, onSave, onClose }: Props = $props();

  const MIB = 1024 * 1024;
  const THRESHOLDS = [1, 2, 5, 10, 25];

  let draft = $state<SettingsDto | null>(null);
  let plain = $state(false);

  // Brouillon repris des paramètres à chaque ouverture.
  $effect(() => {
    if (open && settings !== null) {
      draft = { ...settings };
      plain = plainText;
    }
  });
</script>

<Modal {open} title={m.settings_title()} {onClose} width="520px">
  {#if draft}
    <form
      class="settings"
      onsubmit={(e) => {
        e.preventDefault();
        if (draft !== null) onSave(draft, plain);
      }}
    >
      <label class="row">
        <span>{m.settings_language()}</span>
        <select bind:value={draft.language}>
          <option value="system">{m.settings_language_system()}</option>
          <option value="fr">{m.settings_language_fr()}</option>
          <option value="en">{m.settings_language_en()}</option>
        </select>
      </label>
      <label class="check">
        <input
          type="checkbox"
          checked={draft.update_check === "enabled"}
          onchange={(e) => {
            if (draft !== null)
              draft.update_check = e.currentTarget.checked ? "enabled" : "disabled";
          }}
        />
        {m.settings_updates()}
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={draft.notifications} />
        {m.settings_notifications()}
      </label>
      <label class="check indent">
        <input
          type="checkbox"
          bind:checked={draft.notification_privacy}
          disabled={!draft.notifications}
        />
        {m.settings_privacy()}
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={plain} />
        {m.settings_plain_text()}
      </label>
      <label class="row">
        <span>{m.settings_threshold()}</span>
        <select bind:value={draft.attachment_threshold}>
          {#each THRESHOLDS as size (size)}
            <option value={size * MIB}>{m.settings_threshold_mib({ size })}</option>
          {/each}
        </select>
      </label>
      <p class="note">{m.settings_restart_note()}</p>
      {#if error}
        <p class="error" role="alert">{error}</p>
      {/if}
      <div class="actions">
        <button class="btn" type="button" onclick={onClose}>{m.settings_cancel()}</button>
        <button class="btn btn-primary" type="submit">{m.settings_save()}</button>
      </div>
    </form>
  {/if}
</Modal>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .indent {
    padding-left: var(--space-5, 1.5rem);
  }

  .note {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .error {
    color: var(--color-danger);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
