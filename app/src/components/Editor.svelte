<script lang="ts">
  // Éditeur de rédaction (Tiptap, ADR-010) limité à ce que le Rust garde au
  // réassainissement : paragraphes, gras, italique, barré, liens
  // http/https/mailto, listes, citations. Le HTML produit est traité comme
  // non fiable et réassaini en Rust avant tout envoi.
  import { onMount } from "svelte";
  import { Editor } from "@tiptap/core";
  import StarterKit from "@tiptap/starter-kit";
  import BoldIcon from "@lucide/svelte/icons/bold";
  import ItalicIcon from "@lucide/svelte/icons/italic";
  import StrikeIcon from "@lucide/svelte/icons/strikethrough";
  import LinkIcon from "@lucide/svelte/icons/link";
  import ListIcon from "@lucide/svelte/icons/list";
  import ListOrderedIcon from "@lucide/svelte/icons/list-ordered";
  import QuoteIcon from "@lucide/svelte/icons/text-quote";
  import { formatTooltip } from "../lib/fixed-shortcuts";
  import { m } from "../lib/paraglide/messages.js";

  interface Props {
    /** Contenu initial (lu une seule fois). */
    html: string;
    label: string;
    onChange: (html: string) => void;
  }

  let { html, label, onChange }: Props = $props();

  /** Liens acceptés (comme le réassainissement Rust). */
  const allowedLink = (url: string): boolean => /^(https?:|mailto:)/i.test(url.trim());

  let element: HTMLDivElement;
  let editor = $state<Editor | null>(null);
  /** Incrémenté à chaque transaction : rafraîchit l'état des boutons. */
  let tick = $state(0);
  let linkOpen = $state(false);
  let linkUrl = $state("");

  onMount(() => {
    const instance = new Editor({
      element,
      extensions: [
        StarterKit.configure({
          heading: false,
          code: false,
          codeBlock: false,
          horizontalRule: false,
          underline: false,
          link: {
            openOnClick: false,
            autolink: true,
            defaultProtocol: "https",
            isAllowedUri: (url) => allowedLink(url),
          },
        }),
      ],
      content: html,
      editorProps: {
        attributes: {
          "aria-label": label,
          "aria-multiline": "true",
          role: "textbox",
          class: "editor-body",
        },
      },
      onUpdate: ({ editor: e }) => onChange(e.getHTML()),
      onTransaction: () => {
        tick += 1;
      },
    });
    editor = instance;
    return () => instance.destroy();
  });

  const active = (name: string): boolean => {
    void tick;
    return editor?.isActive(name) ?? false;
  };

  function applyLink(): void {
    const url = linkUrl.trim();
    const chain = editor?.chain().focus().extendMarkRange("link");
    if (url === "") chain?.unsetLink().run();
    else if (allowedLink(url)) chain?.setLink({ href: url }).run();
    linkOpen = false;
    linkUrl = "";
  }
</script>

<div class="editor">
  <div class="toolbar" role="toolbar" aria-label={m.compose_toolbar()}>
    <button
      type="button"
      aria-label={m.fmt_bold()}
      title={formatTooltip("bold")}
      aria-pressed={active("bold")}
      onclick={() => editor?.chain().focus().toggleBold().run()}><BoldIcon size={16} /></button
    >
    <button
      type="button"
      aria-label={m.fmt_italic()}
      title={formatTooltip("italic")}
      aria-pressed={active("italic")}
      onclick={() => editor?.chain().focus().toggleItalic().run()}><ItalicIcon size={16} /></button
    >
    <button
      type="button"
      aria-label={m.fmt_strike()}
      title={formatTooltip("strike")}
      aria-pressed={active("strike")}
      onclick={() => editor?.chain().focus().toggleStrike().run()}><StrikeIcon size={16} /></button
    >
    <button
      type="button"
      aria-label={m.fmt_link()}
      title={formatTooltip("link")}
      aria-pressed={active("link")}
      onclick={() => {
        linkUrl = (editor?.getAttributes("link").href as string | undefined) ?? "";
        linkOpen = !linkOpen;
      }}><LinkIcon size={16} /></button
    >
    <button
      type="button"
      aria-label={m.fmt_bullet()}
      title={formatTooltip("bullet")}
      aria-pressed={active("bulletList")}
      onclick={() => editor?.chain().focus().toggleBulletList().run()}
      ><ListIcon size={16} /></button
    >
    <button
      type="button"
      aria-label={m.fmt_ordered()}
      title={formatTooltip("ordered")}
      aria-pressed={active("orderedList")}
      onclick={() => editor?.chain().focus().toggleOrderedList().run()}
      ><ListOrderedIcon size={16} /></button
    >
    <button
      type="button"
      aria-label={m.fmt_quote()}
      title={formatTooltip("quote")}
      aria-pressed={active("blockquote")}
      onclick={() => editor?.chain().focus().toggleBlockquote().run()}
      ><QuoteIcon size={16} /></button
    >
    {#if linkOpen}
      <form
        class="link"
        onsubmit={(e) => {
          e.preventDefault();
          applyLink();
        }}
      >
        <input type="url" aria-label={m.fmt_link_url()} bind:value={linkUrl} />
        <button class="btn" type="submit">{m.fmt_link_apply()}</button>
      </form>
    {/if}
  </div>
  <div class="surface" bind:this={element}></div>
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    min-height: 240px;
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    padding: var(--space-1);
    border-bottom: var(--border-width) solid var(--color-border);
  }

  .toolbar > button {
    display: inline-flex;
    align-items: center;
    padding: var(--space-1);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }

  .toolbar > button[aria-pressed="true"] {
    background: var(--color-surface-active, var(--color-border));
  }

  .link {
    display: flex;
    gap: var(--space-1);
  }

  .surface {
    flex: 1;
    overflow: auto;
    padding: var(--space-2) var(--space-3);
  }

  .surface :global(.editor-body:focus-visible) {
    box-shadow: inset 0 0 0 2px var(--color-focus);
  }

  .surface :global(.editor-body) {
    min-height: 200px;
    outline: none;
  }

  .surface :global(blockquote) {
    margin: var(--space-2) 0;
    padding-left: var(--space-3);
    border-left: 3px solid var(--color-border);
    color: var(--color-text-muted);
  }
</style>
