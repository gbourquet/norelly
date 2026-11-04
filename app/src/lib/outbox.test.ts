import { describe, expect, it } from "vitest";
import type { SendFailureDto } from "./ipc/api";
import { failureLabel, isScheduled, stateLabel, unixOfLocal } from "./outbox";

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

describe("envoi programmé", () => {
  it("indique l'échéance d'un message programmé", () => {
    const now = 1_800_000_000;
    expect(stateLabel({ state: "queued", not_before: now - 5 }, now)).toBe("En attente");
    expect(stateLabel({ state: "queued", not_before: now + 3600 }, now)).toMatch(
      /^Programmé pour le /,
    );
    expect(isScheduled({ state: "queued", not_before: now + 3600 }, now)).toBe(true);
    expect(isScheduled({ state: "queued", not_before: now + 30 }, now)).toBe(false);
    expect(isScheduled({ state: "sending" }, now)).toBe(false);
  });

  it("convertit la date et l'heure saisies en secondes Unix", () => {
    expect(unixOfLocal("")).toBeNull();
    expect(unixOfLocal("pas une date")).toBeNull();
    const local = "2026-12-24T18:30";
    expect(unixOfLocal(local)).toBe(Math.floor(new Date(local).getTime() / 1000));
  });
});
