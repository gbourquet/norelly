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

  interface Props {
    open: boolean;
    registry: CommandRegistry<C>;
    onClose: () => void;
  }

  let { open, registry, onClose }: Props = $props();

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
  <p class="hint">
    {m.help_palette_hint()} <kbd>Ctrl</kbd> <kbd>K</kbd>
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
    margin-top: var(--space-4);
    color: var(--color-text-muted);
  }
</style>
