<script lang="ts" generics="C">
  import { m } from "../lib/paraglide/messages.js";
  // Aide des raccourcis (« ? »), générée depuis le registre.
  import Modal from "./Modal.svelte";
  import { formatShortcut } from "../lib/commands/shortcut";
  import {
    groupCommands,
    type CommandRegistry,
    type RegisteredCommand,
  } from "../lib/commands/registry";
  import type { Shortcut } from "../lib/commands/shortcut";
  import { composeWindowShortcuts } from "../lib/fixed-shortcuts";
  import { describeCommand } from "../lib/commands/describe";

  interface Props {
    open: boolean;
    registry: CommandRegistry<C>;
    context: C;
    onClose: () => void;
    /** Ouvre la palette de commandes (bouton de l'aide). */
    onPalette: () => void;
  }

  let { open, registry, context, onClose, onPalette }: Props = $props();

  const palette = $derived(open ? describeCommand(registry, "palette.open", context) : null);

  type WithShortcut = RegisteredCommand<C> & { parsedShortcut: Shortcut };
  const hasShortcut = (c: RegisteredCommand<C>): c is WithShortcut =>
    c.parsedShortcut !== undefined;
  // Recalculé à chaque ouverture (le registre n'est pas réactif).
  const groups = $derived(open ? groupCommands(registry.all().filter(hasShortcut)) : []);
</script>

<Modal {open} title={m.help_title()} {onClose} width="520px">
  {#each groups as [group, commands] (group)}
    <h3>{group}</h3>
    <dl>
      {#each commands as command (command.id)}
        <div class="row">
          <dt>{command.title}</dt>
          <dd>
            {#each formatShortcut(command.parsedShortcut) as k (k)}<kbd>{k}</kbd>{/each}
          </dd>
        </div>
      {/each}
    </dl>
  {/each}
  <h3>{m.help_group_compose_window()}</h3>
  <dl>
    {#each composeWindowShortcuts() as shortcut (shortcut.label)}
      <div class="row">
        <dt>{shortcut.label}</dt>
        <dd>
          {#each shortcut.keys as k (k)}<kbd>{k}</kbd>{/each}
        </dd>
      </div>
    {/each}
  </dl>
  <p class="hint">
    {m.help_palette_hint()}
    <button class="btn" type="button" title={palette?.tooltip} onclick={onPalette}>
      {palette?.label}
    </button>
  </p>
</Modal>

<style>
  h3 {
    margin: var(--space-3) 0 var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    text-transform: uppercase;
  }

  dl {
    margin: 0;
  }

  .row {
    display: flex;
    justify-content: space-between;
    padding: var(--space-1) 0;
    border-bottom: var(--border-width) solid var(--color-border);
  }

  dt,
  dd {
    margin: 0;
  }

  dd {
    display: flex;
    gap: var(--space-1);
  }

  .hint {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-4);
    color: var(--color-text-muted);
  }

  .hint .btn {
    color: var(--color-text);
  }
</style>
