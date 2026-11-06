<script lang="ts">
  // Choix d'un dossier (déplacer, dossier d'archives), au clavier :
  // palette filtrée par le nom (bits-ui Command).
  import { Command, Dialog } from "bits-ui";
  import XIcon from "@lucide/svelte/icons/x";
  import FolderIcon from "@lucide/svelte/icons/folder";
  import type { FolderDto } from "../lib/ipc/api";
  import { m } from "../lib/paraglide/messages.js";

  interface Props {
    open: boolean;
    title: string;
    folders: FolderDto[];
    onChoose: (folder: FolderDto) => void;
    onClose: () => void;
  }

  let { open, title, folders, onChoose, onClose }: Props = $props();

  let query = $state("");
  const results = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return folders.filter((f) => f.selectable && (q === "" || f.name.toLowerCase().includes(q)));
  });

  function choose(folder: FolderDto): void {
    query = "";
    onChoose(folder);
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
    <Dialog.Content class="palette" aria-label={title}>
      <Dialog.Title class="visually-hidden">{title}</Dialog.Title>
      <Command.Root shouldFilter={false} label={title}>
        <div class="palette-search">
          <FolderIcon size={16} aria-hidden="true" />
          <Command.Input
            class="palette-input"
            placeholder={m.picker_placeholder()}
            bind:value={query}
          />
          <Dialog.Close
            class="btn btn-ghost palette-close"
            aria-label={m.common_close()}
            title={m.common_close()}
          >
            <XIcon size={16} aria-hidden="true" />
          </Dialog.Close>
        </div>
        <Command.List class="palette-list">
          <Command.Empty class="palette-empty">{m.picker_empty()}</Command.Empty>
          {#each results as folder (folder.id)}
            <Command.Item
              class="palette-item"
              value={String(folder.id)}
              onSelect={() => choose(folder)}
            >
              <span style:padding-left="{Math.min(folder.depth, 6) * 12}px">{folder.name}</span>
            </Command.Item>
          {/each}
        </Command.List>
      </Command.Root>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>
