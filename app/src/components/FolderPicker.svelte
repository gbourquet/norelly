<script lang="ts">
  // Choix d'un dossier (déplacer, dossier d'archives), au clavier ou à la
  // souris : palette filtrée par le libellé ou le nom du serveur (bits-ui
  // Command) ; « Nouveau dossier… » en dernier si `onCreate` est fourni.
  import { Command, Dialog } from "bits-ui";
  import XIcon from "@lucide/svelte/icons/x";
  import FolderIcon from "@lucide/svelte/icons/folder";
  import FolderPlusIcon from "@lucide/svelte/icons/folder-plus";
  import type { FolderDto } from "../lib/ipc/api";
  import { m } from "../lib/paraglide/messages.js";
  import { folderLabel } from "../lib/folder-label";

  interface Props {
    open: boolean;
    title: string;
    folders: FolderDto[];
    onChoose: (folder: FolderDto) => void;
    onClose: () => void;
    /** Création d'un dossier proposée, avec le texte cherché comme nom. */
    onCreate?: (name: string) => void;
  }

  let { open, title, folders, onChoose, onClose, onCreate }: Props = $props();

  let query = $state("");
  const results = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return folders.filter(
      (f) =>
        f.selectable &&
        (q === "" || f.name.toLowerCase().includes(q) || label(f).toLowerCase().includes(q)),
    );
  });

  /** Dossier spécial : libellé traduit ; autre : nom complet du serveur. */
  function label(folder: FolderDto): string {
    return folder.role === "normal" ? folder.name : folderLabel(folder);
  }

  function choose(folder: FolderDto): void {
    query = "";
    onChoose(folder);
  }

  /** Nouveau dossier, nommé d'après la recherche, sauf si elle désigne
   * déjà un dossier proposé (le nom serait refusé comme doublon). */
  function create(create: (name: string) => void): void {
    const name = query.trim();
    const lower = name.toLowerCase();
    const existing = results.some(
      (f) => f.leaf.toLowerCase() === lower || label(f).toLowerCase() === lower,
    );
    query = "";
    create(existing ? "" : name);
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
          {#if results.length === 0}
            <div class="palette-empty">{m.picker_empty()}</div>
          {/if}
          {#each results as folder (folder.id)}
            <Command.Item
              class="palette-item"
              value={String(folder.id)}
              onSelect={() => choose(folder)}
            >
              <span>{label(folder)}</span>
            </Command.Item>
          {/each}
          {#if onCreate}
            <Command.Separator class="menu-separator" />
            <Command.Item
              class="palette-item palette-create"
              value="new-folder"
              onSelect={() => create(onCreate)}
            >
              <FolderPlusIcon size={16} aria-hidden="true" />
              <span>{m.picker_new_folder()}</span>
            </Command.Item>
          {/if}
        </Command.List>
      </Command.Root>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  /* « Nouveau dossier… » : icône puis libellé, à gauche. */
  :global(.palette-item.palette-create) {
    justify-content: flex-start;
    gap: var(--space-2);
  }
</style>
