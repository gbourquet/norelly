import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/svelte";
import { afterEach } from "vitest";

// Démonte les composants entre deux tests (globals vitest désactivés).
afterEach(() => cleanup());

// jsdom n'implémente pas scrollIntoView (utilisé par la palette bits-ui).
if (typeof Element.prototype.scrollIntoView !== "function") {
  Element.prototype.scrollIntoView = function scrollIntoView(): void {};
}

// jsdom n'implémente pas ResizeObserver (liaisons de largeur Svelte) : les
// largeurs restent nulles, les barres d'actions affichent tout.
if (typeof globalThis.ResizeObserver !== "function") {
  globalThis.ResizeObserver = class ResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  };
}

// jsdom n'implémente pas PointerEvent (glisser-déposer des conversations) :
// un MouseEvent (position, bouton) avec le type de pointeur.
if (typeof globalThis.PointerEvent !== "function") {
  globalThis.PointerEvent = class PointerEvent extends MouseEvent {
    readonly pointerType: string;
    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init);
      this.pointerType = init.pointerType ?? "mouse";
    }
  } as unknown as typeof globalThis.PointerEvent;
}
