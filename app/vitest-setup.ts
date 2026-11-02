import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/svelte";
import { afterEach } from "vitest";

// Démonte les composants entre deux tests (globals vitest désactivés).
afterEach(() => cleanup());

// jsdom n'implémente pas scrollIntoView (utilisé par la palette bits-ui).
if (typeof Element.prototype.scrollIntoView !== "function") {
  Element.prototype.scrollIntoView = function scrollIntoView(): void {};
}
