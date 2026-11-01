import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/svelte";
import App from "./App.svelte";

describe("App", () => {
  it("affiche le nom de l'application", () => {
    render(App);
    expect(screen.getByRole("heading", { name: "Norelly" })).toBeInTheDocument();
  });
});
