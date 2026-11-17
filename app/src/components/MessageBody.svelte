<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  import { errorText } from "../lib/errors";
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
      error = errorText(e);
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
          notice = m.message_link_failed({ error: errorText(e) });
        });
        break;
      case "key":
        onKey(parsed.shortcut);
        break;
    }
  }

  // Impression : le cadre garde la largeur de l'écran, pour que la hauteur
  // annoncée par l'agent reste juste (aucun script ne tourne pendant la
  // mise en page du papier). Plus large que la feuille, la page est réduite
  // à sa largeur par le moteur (WebKitGTK et WebView2 le font par défaut).
  // Réglé directement sur l'élément : l'impression suit l'événement sans
  // attendre une mise à jour de Svelte.
  function keepWidthForPrint(): void {
    if (frame !== null && frame.clientWidth > 0) frame.style.width = `${frame.clientWidth}px`;
  }

  function releasePrintWidth(): void {
    if (frame !== null) frame.style.width = "";
  }

  async function reloadAfter(action: () => Promise<void>): Promise<void> {
    try {
      await action();
      await load();
      version += 1;
    } catch (e) {
      notice = errorText(e);
    }
  }

  async function attachment(part: string, open: boolean): Promise<void> {
    notice = null;
    try {
      if (open) {
        await api.openAttachment(message, part);
      } else {
        const saved = await api.saveAttachment(message, part, m.message_save_dialog_title());
        if (saved !== null) notice = m.message_saved_as({ path: saved });
      }
      await load();
    } catch (e) {
      notice = errorText(e);
    }
  }
</script>

<svelte:window
  onmessage={onMessage}
  onbeforeprint={keepWidthForPrint}
  onafterprint={releasePrintWidth}
/>

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
    {#if view.notice}
      <p class="notice" role="status">
        {view.notice === "too_complex"
          ? m.message_notice_too_complex()
          : view.notice === "too_long"
            ? m.message_notice_too_long()
            : m.message_notice_not_available()}
      </p>
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
      <p class="notice transient" role="status">{notice}</p>
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

  /* Papier : le corps, l'avis d'affichage simplifié et le nom des pièces
     jointes, sans les boutons ni les messages d'une action. En bloc : les
     moteurs coupent mal une colonne flexible entre deux pages. */
  @media print {
    .body {
      display: block;
    }

    .banner,
    .mode,
    .transient,
    .attachments button {
      display: none;
    }
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
