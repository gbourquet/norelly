// Mesure de bout en bout, lancée seulement avec
// NORELLY_E2E_PERF=1 (job nocturne) : base pré-remplie de 50 000 messages
// dans la boîte de réception et 20 000 dans Archives (exemple
// `seed_perf` de norelly-sqlite), compte sur un domaine injoignable.
//
// - Démarrage : du chargement de la page au premier affichage d'une liste
//   (repère `norelly:first-list` posé par l'interface), < 400 ms.
// - Changement de dossier : du clic à l'affichage de la nouvelle liste
//   (repère `norelly:list-shown`), médiane de 11 changements, < 200 ms.
import { $$, browser, expect } from "@wdio/globals";

/** Seuil du démarrage, fixé par le mainteneur le 2026-11-04 (mesure sous
 * Xvfb, WebKitGTK sans accélération graphique). */
const STARTUP_THRESHOLD_MS = 400;
/** Seuil du changement de dossier. */
const THRESHOLD_MS = 200;

/** Durée d'un changement vers le dossier de libellé `label`. */
async function switchTo(label) {
  return browser.execute(
    (wanted) =>
      new Promise((resolve) => {
        const button = [...document.querySelectorAll("button.folder")].find(
          (b) => b.querySelector(".label")?.textContent?.trim() === wanted,
        );
        if (!button) {
          resolve(-1);
          return;
        }
        const before = performance.getEntriesByName("norelly:list-shown").length;
        const started = performance.now();
        button.click();
        const poll = () => {
          if (performance.getEntriesByName("norelly:list-shown").length > before) {
            resolve(performance.now() - started);
          } else {
            requestAnimationFrame(poll);
          }
        };
        poll();
      }),
    label,
  );
}

describe("performance sur une boîte de 50 000 messages", () => {
  it("affiche la première liste en moins de 400 ms", async () => {
    await $$('[role="option"]')[0].waitForDisplayed({ timeout: 60000 });
    const started = await browser.execute(
      () => performance.getEntriesByName("norelly:first-list")[0]?.startTime ?? -1,
    );
    console.log(`démarrage : ${started.toFixed(1)} ms (seuil ${STARTUP_THRESHOLD_MS} ms)`);
    const detail = await browser.execute(() => {
      const nav = performance.getEntriesByType("navigation")[0];
      const marks = performance
        .getEntriesByType("mark")
        .map((m) => `${m.name}=${m.startTime.toFixed(1)}`);
      return {
        responseEnd: nav?.responseEnd,
        domInteractive: nav?.domInteractive,
        domContentLoaded: nav?.domContentLoadedEventEnd,
        marks,
      };
    });
    console.log(`détail du démarrage : ${JSON.stringify(detail)}`);
    const atRest = await browser.execute(async () => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      const time = async (cmd, args) => {
        const t = performance.now();
        await invoke(cmd, args);
        return Math.round(performance.now() - t);
      };
      return {
        get_settings: await time("get_settings", {}),
        list_accounts: await time("list_accounts", {}),
        list_accounts_again: await time("list_accounts", {}),
        list_folders: await time("list_folders", { account: 1 }),
        list_conversations: await time("list_conversations", {
          scope: { kind: "unified", role: "inbox" },
          cursor: null,
          limit: 100,
        }),
      };
    });
    console.log(`appels au repos (ms) : ${JSON.stringify(atRest)}`);
    expect(started).toBeGreaterThan(0);
    expect(started).toBeLessThan(STARTUP_THRESHOLD_MS);
  });

  it("change de dossier en moins de 200 ms (médiane)", async () => {
    const times = [];
    for (let i = 0; i < 11; i++) {
      times.push(await switchTo(i % 2 === 0 ? "Archives" : "Boîte de réception"));
    }
    expect(times.every((t) => t >= 0)).toBe(true);
    times.sort((a, b) => a - b);
    const median = times[Math.floor(times.length / 2)];
    console.log(
      `changement de dossier : médiane ${median.toFixed(1)} ms (seuil ${THRESHOLD_MS} ms)`,
    );
    expect(median).toBeLessThan(THRESHOLD_MS);
  });
});
