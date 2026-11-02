// Liste virtualisée maison (ADR-005) au-dessus de @tanstack/virtual-core :
// seules les lignes visibles (plus une marge) sont dans le DOM, quelle que
// soit la taille de la liste. Hauteur de ligne fixe.

import {
  Virtualizer,
  elementScroll,
  observeElementOffset,
  observeElementRect,
  type VirtualItem,
} from "@tanstack/virtual-core";

export class VirtualList {
  items = $state<VirtualItem[]>([]);
  totalSize = $state(0);
  readonly #virtualizer: Virtualizer<HTMLElement, Element>;

  constructor(getScrollElement: () => HTMLElement | null, rowHeight: number, overscan = 8) {
    // Panneau pas encore dimensionné (ou environnement sans mise en page,
    // comme jsdom) : une fenêtre de 20 lignes par défaut.
    const fallback = { width: 400, height: rowHeight * 20 };
    this.#virtualizer = new Virtualizer<HTMLElement, Element>({
      count: 0,
      getScrollElement,
      estimateSize: () => rowHeight,
      scrollToFn: elementScroll,
      observeElementRect: (instance, cb) =>
        observeElementRect(instance, (rect) => cb(rect.height > 0 ? rect : fallback)),
      observeElementOffset,
      overscan,
      initialRect: fallback,
      onChange: (instance) => this.#refresh(instance),
    });
  }

  #refresh(instance: Virtualizer<HTMLElement, Element>): void {
    this.items = instance.getVirtualItems();
    this.totalSize = instance.getTotalSize();
  }

  /** À appeler au montage ; renvoie la fonction de nettoyage. */
  mount(): () => void {
    const cleanup = this.#virtualizer._didMount();
    this.#virtualizer._willUpdate();
    this.#refresh(this.#virtualizer);
    return cleanup;
  }

  setCount(count: number): void {
    if (this.#virtualizer.options.count === count) return;
    this.#virtualizer.setOptions({ ...this.#virtualizer.options, count });
    this.#virtualizer._willUpdate();
    this.#refresh(this.#virtualizer);
  }

  scrollToIndex(index: number): void {
    if (index >= 0) this.#virtualizer.scrollToIndex(index, { align: "auto" });
  }
}
