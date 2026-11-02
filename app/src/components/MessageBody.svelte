<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Corps d'un message (ADR-004, « A+ ») : iframe mailview:// sandboxée
  // (allow-scripts sans allow-same-origin : origine opaque), dont seul
  // l'agent peut s'exécuter (CSP par empreinte). L'agent signale hauteur,
  // liens et touches par postMessage ; chaque message est validé
  // (parseAgentMessage) et les liens sont confiés au Rust (open_external).
  import ImageOffIcon from "@lucide/svelte/icons/image-off";
  import PaperclipIcon from "@lucide/svelte/icons/paperclip";
  import type { Api, MessageViewDto } from "../lib/ipc/api";
  import type { Shortcut } from "../lib/commands/shortcut";
  import { mailviewUrl, parseAgentMessage } from "../lib/agent";
  import { fileSize } from "../lib/format";

  interface Props {
    message: number;
    api: Pick<
      Api,
      | "openMessage"
      | "allowRemoteImages"
      | "setPreferPlainText"
      | "openExternal"
      | "openAttachment"
      | "saveAttachment"
    >;
    onKey: (shortcut: Shortcut) => void;
  }

  let { message, api, onKey }: Props = $props();

  let view = $state<MessageViewDto | null>(null);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let height = $state(80);
  let version = $state(0);
  let frame: HTMLIFrameElement | null = $state(null);

  async function load(): Promise<void> {
    try {
      view = await api.openMessage(message);
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  $effect(() => {
    void message;
    view = null;
    void load();
  });

  function onMessage(event: MessageEvent): void {
    const parsed = parseAgentMessage(event, frame?.contentWindow);
    if (parsed === null) return;
    switch (parsed.type) {
      case "height":
        height = Math.max(parsed.height, 40);
        break;
      case "link":
        api.openExternal(parsed.href).catch((e: unknown) => {
          notice = m.message_link_failed({ error: e instanceof Error ? e.message : String(e) });
        });
        break;
      case "key":
        onKey(parsed.shortcut);
        break;
    }
  }

  async function reloadAfter(action: () => Promise<void>): Promise<void> {
    try {
      await action();
      await load();
      version += 1;
    } catch (e) {
      notice = e instanceof Error ? e.message : String(e);
    }
  }

  async function attachment(part: string, open: boolean): Promise<void> {
    notice = null;
    try {
      if (open) {
        await api.openAttachment(message, part);
      } else {
        const saved = await api.saveAttachment(message, part);
        if (saved !== null) notice = m.message_saved_as({ path: saved });
      }
      await load();
    } catch (e) {
      notice = e instanceof Error ? e.message : String(e);
    }
  }
</script>

<svelte:window onmessage={onMessage} />

<div class="body">
  {#if error}
    <p class="error" role="alert">{m.message_unreadable({ error })}</p>
  {:else if view === null}
    <p class="loading">{m.common_loading()}</p>
  {:else}
    {#if view.blocked_images > 0}
      <div class="banner" role="status">
        <ImageOffIcon size={16} aria-hidden="true" />
        <span>{m.message_images_blocked({ count: view.blocked_images })}</span>
        <button
          class="btn"
          type="button"
          onclick={() => reloadAfter(() => api.allowRemoteImages(message))}
          >{m.message_show_images()}</button
        >
      </div>
    {/if}
    {#if view.has_html}
      <div class="mode">
        <button
          class="btn btn-ghost"
          type="button"
          onclick={() => reloadAfter(() => api.setPreferPlainText(view?.mode !== "text"))}
          >{view.mode === "text" ? m.message_show_html() : m.message_show_text()}</button
        >
      </div>
    {/if}
    <iframe
      bind:this={frame}
      class="frame"
      title={m.message_frame_title()}
      src={mailviewUrl(message, version)}
      sandbox="allow-scripts"
      referrerpolicy="no-referrer"
      style:height="{height}px"
    ></iframe>
    {#if view.attachments.length > 0}
      <ul class="attachments" aria-label={m.message_attachments()}>
        {#each view.attachments as a (a.part)}
          <li>
            <PaperclipIcon size={14} aria-hidden="true" />
            <span class="name" title={a.media_type}>{a.name}</span>
            <span class="size">{fileSize(a.size)}{a.available ? "" : m.message_to_download()}</span>
            {#if a.can_open}
              <button class="btn btn-ghost" type="button" onclick={() => attachment(a.part, true)}
                >{m.message_open()}</button
              >
            {/if}
            <button class="btn btn-ghost" type="button" onclick={() => attachment(a.part, false)}
              >{m.message_save()}</button
            >
          </li>
        {/each}
      </ul>
    {/if}
    {#if notice}
      <p class="notice" role="status">{notice}</p>
    {/if}
  {/if}
</div>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .frame {
    width: 100%;
    border: none;
    border-radius: var(--radius);
    background: #fff;
  }

  .banner {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius);
    background: var(--color-warning-bg);
    color: var(--color-warning);
    font-size: var(--font-size-sm);
  }

  .mode {
    display: flex;
    justify-content: flex-end;
  }

  .attachments {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .attachments li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) 0;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size,
  .loading,
  .notice {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .error {
    color: var(--color-danger);
  }
</style>
