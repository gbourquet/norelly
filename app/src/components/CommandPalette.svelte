<script lang="ts" generics="C">
  // Palette Ctrl+K (bits-ui Command), alimentée par le registre central.
  import { Command, Dialog } from "bits-ui";
  import SearchIcon from "@lucide/svelte/icons/search";
  import { formatShortcut } from "../lib/commands/shortcut";
  import { groupCommands, type CommandRegistry } from "../lib/commands/registry";

  interface Props {
    open: boolean;
    registry: CommandRegistry<C>;
    context: C;
    onClose: () => void;
  }

  let { open, registry, context, onClose }: Props = $props();

  let query = $state("");
  const results = $derived(open ? registry.search(context, query) : []);
  const groups = $derived(groupCommands(results));

  function run(id: string): void {
    query = "";
    onClose();
    void registry.execute(id, context);
  }
</script>

<Dialog.Root
  {open}
  onOpenChange={(o) => {
    if (!o) {
      query = "";
      onClose();
    }
  }}
>
  <Dialog.Portal>
    <Dialog.Overlay class="modal-overlay" />
    <Dialog.Content class="palette" aria-label="Palette de commandes">
      <Dialog.Title class="visually-hidden">Palette de commandes</Dialog.Title>
      <Command.Root shouldFilter={false} label="Commandes">
        <div class="palette-search">
          <SearchIcon size={16} aria-hidden="true" />
          <Command.Input
            class="palette-input"
            placeholder="Rechercher une commande…"
            bind:value={query}
          />
        </div>
        <Command.List class="palette-list">
          <Command.Empty class="palette-empty">Aucune commande</Command.Empty>
          {#each groups as [group, commands] (group)}
            <Command.Group>
              <Command.GroupHeading class="palette-group">{group}</Command.GroupHeading>
              <Command.GroupItems>
                {#each commands as command (command.id)}
                  <Command.Item
                    class="palette-item"
                    value={command.id}
                    onSelect={() => run(command.id)}
                  >
                    <span>{command.title}</span>
                    {#if command.parsedShortcut}
                      <span class="palette-keys">
                        {#each formatShortcut(command.parsedShortcut) as k (k)}<kbd>{k}</kbd>{/each}
                      </span>
                    {/if}
                  </Command.Item>
                {/each}
              </Command.GroupItems>
            </Command.Group>
          {/each}
        </Command.List>
      </Command.Root>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  :global(.palette) {
    position: fixed;
    top: 14vh;
    left: 50%;
    z-index: 51;
    width: min(560px, calc(100vw - 2 * var(--space-4)));
    transform: translateX(-50%);
    overflow: hidden;
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-dialog);
  }

  .palette-search {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    border-bottom: var(--border-width) solid var(--color-border);
    color: var(--color-text-muted);
  }

  :global(.palette-input) {
    flex: 1;
    border: none;
    outline: none;
    background: transparent;
    font-size: var(--font-size-lg);
  }

  :global(.palette-list) {
    max-height: 50vh;
    overflow: auto;
    padding: var(--space-2);
  }

  :global(.palette-empty) {
    padding: var(--space-4);
    color: var(--color-text-muted);
    text-align: center;
  }

  :global(.palette-group) {
    padding: var(--space-2) var(--space-2) var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    text-transform: uppercase;
  }

  :global(.palette-item) {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: var(--space-2);
    border-radius: var(--radius);
    cursor: pointer;
  }

  :global(.palette-item[data-selected]) {
    background: var(--color-selected);
  }

  .palette-keys {
    display: flex;
    gap: var(--space-1);
  }
</style>
