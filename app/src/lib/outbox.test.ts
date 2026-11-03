import { describe, expect, it } from "vitest";
import type { SendFailureDto } from "./ipc/api";
import { failureLabel, stateLabel } from "./outbox";

describe("libellés de la file d'envoi", () => {
  it("donne un libellé à chaque état et chaque cause d'échec", () => {
    expect(stateLabel({ state: "queued", not_before: 0 })).toBe("En attente");
    expect(stateLabel({ state: "sending" })).toBe("Envoi en cours");
    expect(stateLabel({ state: "sent" })).toBe("Envoyé");
    expect(stateLabel({ state: "failed", failure: "uncertain" })).toBe(
      "envoi interrompu, peut-être parti",
    );
    const failures: SendFailureDto[] = [
      "unreachable",
      "temporary",
      "rejected",
      "authentication",
      "security",
      "uncertain",
      "too_many_attempts",
    ];
    const labels = failures.map(failureLabel);
    expect(new Set(labels).size).toBe(failures.length);
    for (const l of labels) expect(l).not.toBe("");
  });
});
