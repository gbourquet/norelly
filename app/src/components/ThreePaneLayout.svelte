<script lang="ts">
  // Mise en page en 3 panneaux redimensionnables : navigation, liste, lecture.
  // Les largeurs sont mémorisées localement (confort, non critique).
  import type { Snippet } from "svelte";
  import Splitter from "./Splitter.svelte";

  interface Props {
    sidebar: Snippet;
    list: Snippet;
    reader: Snippet;
  }

  let { sidebar, list, reader }: Props = $props();

  const STORAGE_KEY = "norelly.layout.v1";
  const SIDEBAR = { min: 160, max: 420, initial: 240 };
  const LIST = { min: 240, max: 720, initial: 380 };

  function load(): { sidebar: number; list: number } {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      const parsed: unknown = raw === null ? null : JSON.parse(raw);
      if (parsed !== null && typeof parsed === "object") {
        const p = parsed as Record<string, unknown>;
        const ok = (v: unknown, b: { min: number; max: number }): v is number =>
          typeof v === "number" && v >= b.min && v <= b.max;
        if (ok(p["sidebar"], SIDEBAR) && ok(p["list"], LIST))
          return { sidebar: p["sidebar"], list: p["list"] };
      }
    } catch {
      // Stockage indisponible ou valeur illisible : largeurs par défaut.
    }
    return { sidebar: SIDEBAR.initial, list: LIST.initial };
  }

  const sizes = $state(load());

  function save(): void {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(sizes));
    } catch {
      // Sans importance.
    }
  }
</script>

<div class="layout" style="--w-sidebar: {sizes.sidebar}px; --w-list: {sizes.list}px">
  <nav class="pane sidebar" aria-label="Comptes et dossiers">{@render sidebar()}</nav>
  <Splitter
    label="Largeur du panneau de navigation"
    value={sizes.sidebar}
    min={SIDEBAR.min}
    max={SIDEBAR.max}
    onChange={(v) => {
      sizes.sidebar = v;
      save();
    }}
  />
  <section class="pane list" aria-label="Liste des messages">{@render list()}</section>
  <Splitter
    label="Largeur de la liste des messages"
    value={sizes.list}
    min={LIST.min}
    max={LIST.max}
    onChange={(v) => {
      sizes.list = v;
      save();
    }}
  />
  <main class="pane reader" aria-label="Lecture">{@render reader()}</main>
</div>

<style>
  .layout {
    display: flex;
    height: 100%;
    min-width: 0;
  }

  .pane {
    min-width: 0;
    overflow: auto;
  }

  .sidebar {
    flex: 0 0 var(--w-sidebar);
    background: var(--color-surface);
  }

  .list {
    flex: 0 0 var(--w-list);
  }

  .reader {
    flex: 1 1 auto;
  }
</style>
