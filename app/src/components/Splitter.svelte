<script lang="ts">
  // Séparateur redimensionnable entre deux panneaux (souris et clavier).
  interface Props {
    label: string;
    value: number;
    min: number;
    max: number;
    onChange: (value: number) => void;
  }

  let { label, value, min, max, onChange }: Props = $props();

  const STEP = 16;
  let start: { x: number; value: number } | null = null;

  const clamp = (v: number): number => Math.min(max, Math.max(min, Math.round(v)));

  function onPointerDown(e: PointerEvent): void {
    if (e.button !== 0) return;
    start = { x: e.clientX, value };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function onPointerMove(e: PointerEvent): void {
    if (start !== null) onChange(clamp(start.value + e.clientX - start.x));
  }

  function onPointerUp(e: PointerEvent): void {
    start = null;
    const target = e.currentTarget as HTMLElement;
    if (target.hasPointerCapture(e.pointerId)) target.releasePointerCapture(e.pointerId);
  }

  function onKeyDown(e: KeyboardEvent): void {
    const delta = { ArrowLeft: -STEP, ArrowRight: STEP }[e.key];
    if (delta !== undefined) {
      onChange(clamp(value + delta));
      e.preventDefault();
    } else if (e.key === "Home") {
      onChange(min);
      e.preventDefault();
    } else if (e.key === "End") {
      onChange(max);
      e.preventDefault();
    }
  }
</script>

<!-- Motif ARIA « window splitter » : un séparateur focalisable est un widget
     interactif, ce que la règle a11y de Svelte ne reconnaît pas. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="splitter"
  role="separator"
  aria-label={label}
  aria-orientation="vertical"
  aria-valuenow={value}
  aria-valuemin={min}
  aria-valuemax={max}
  tabindex="0"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
  onkeydown={onKeyDown}
></div>

<style>
  .splitter {
    flex: 0 0 var(--splitter-size);
    cursor: col-resize;
    background: var(--color-border);
    touch-action: none;
  }

  .splitter:hover,
  .splitter:focus-visible {
    background: var(--color-accent);
  }
</style>
