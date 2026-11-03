import { beforeEach, describe, expect, it } from "vitest";
import { createRawSnippet } from "svelte";
import { fireEvent, render, screen } from "@testing-library/svelte";
import ThreePaneLayout from "./ThreePaneLayout.svelte";

const KEY = "norelly.layout.v1";
const pane = (text: string) => createRawSnippet(() => ({ render: () => `<p>${text}</p>` }));

function show() {
  return render(ThreePaneLayout, {
    sidebar: pane("navigation"),
    list: pane("liste"),
    reader: pane("lecture"),
  });
}

const width = (name: string): number =>
  Number(screen.getByRole("separator", { name }).getAttribute("aria-valuenow"));

describe("mise en page en trois panneaux", () => {
  beforeEach(() => localStorage.clear());

  it("relit les largeurs mémorisées valides", () => {
    localStorage.setItem(KEY, JSON.stringify({ sidebar: 300, list: 500 }));
    show();
    expect(width("Largeur du panneau de navigation")).toBe(300);
    expect(width("Largeur de la liste des messages")).toBe(500);
  });

  it("ignore une valeur hors bornes", () => {
    localStorage.setItem(KEY, JSON.stringify({ sidebar: 9999, list: 500 }));
    show();
    expect(width("Largeur du panneau de navigation")).toBe(240);
    expect(width("Largeur de la liste des messages")).toBe(380);
  });

  it("ignore une valeur illisible", () => {
    localStorage.setItem(KEY, "{pas du json");
    show();
    expect(width("Largeur de la liste des messages")).toBe(380);
  });

  it("mémorise un redimensionnement au clavier", async () => {
    show();
    await fireEvent.keyDown(
      screen.getByRole("separator", { name: "Largeur de la liste des messages" }),
      {
        key: "ArrowRight",
      },
    );
    expect(JSON.parse(localStorage.getItem(KEY) ?? "{}")).toEqual({ sidebar: 240, list: 396 });
  });
});
