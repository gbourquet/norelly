import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/svelte";
import ServerFields from "./ServerFields.svelte";
import type { ServerDto } from "../lib/ipc/api";

function server(port: number): ServerDto {
  return { host: "smtp.example.com", port, security: "tls", username: "alice" };
}

describe("champs d'un serveur", () => {
  it("suit le port par défaut de la sécurité choisie", async () => {
    const value = server(465);
    render(ServerFields, { legend: "Envoi", idPrefix: "smtp", server: value });
    const security = screen.getByLabelText("Sécurité");
    // Le parent lie l'objet (`bind:server`) : c'est lui qui est mis à jour.
    await fireEvent.change(security, { target: { value: "starttls" } });
    expect(value).toMatchObject({ security: "starttls", port: 587 });
    await fireEvent.change(security, { target: { value: "plain" } });
    expect(value).toMatchObject({ security: "plain", port: 25 });
  });

  it("garde un port personnalisé", async () => {
    const value = server(31993);
    render(ServerFields, { legend: "Réception", idPrefix: "imap", server: value });
    await fireEvent.change(screen.getByLabelText("Sécurité"), { target: { value: "starttls" } });
    expect(value).toMatchObject({ security: "starttls", port: 31993 });
  });

  it("désactive tous les champs", () => {
    render(ServerFields, {
      legend: "Réception",
      idPrefix: "imap",
      server: server(993),
      disabled: true,
    });
    expect(screen.getByRole("group", { name: "Réception" })).toBeDisabled();
  });
});
