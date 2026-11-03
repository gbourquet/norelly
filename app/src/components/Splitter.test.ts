import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import Splitter from "./Splitter.svelte";

describe("séparateur", () => {
  it("se déplace au clavier, dans ses bornes", async () => {
    const onChange = vi.fn();
    render(Splitter, { label: "Largeur", value: 100, min: 50, max: 120, onChange });
    const sep = screen.getByRole("separator", { name: "Largeur" });
    await fireEvent.keyDown(sep, { key: "ArrowLeft" });
    expect(onChange).toHaveBeenLastCalledWith(84);
    await fireEvent.keyDown(sep, { key: "ArrowRight" });
    expect(onChange).toHaveBeenLastCalledWith(116);
    await fireEvent.keyDown(sep, { key: "Home" });
    expect(onChange).toHaveBeenLastCalledWith(50);
    await fireEvent.keyDown(sep, { key: "End" });
    expect(onChange).toHaveBeenLastCalledWith(120);
    onChange.mockClear();
    await fireEvent.keyDown(sep, { key: "a" });
    expect(onChange).not.toHaveBeenCalled();
  });
});
