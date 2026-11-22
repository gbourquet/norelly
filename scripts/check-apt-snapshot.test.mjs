// Tests du contrôle de l'instantané apt de l'image e2e
// (scripts/check-apt-snapshot.mjs), lancés par scripts/check.sh avec
// `node --test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { snapshotStatus, MAX_AGE_DAYS } from "./check-apt-snapshot.mjs";

const dockerfile = (line) =>
  `FROM ubuntu:24.04@sha256:abc\nRUN ${line} \\\n    && apt-get update\n`;
const day = (iso) => new Date(`${iso}T12:00:00Z`);

test("lit la date de l'instantané et son âge en jours", () => {
  assert.deepEqual(
    snapshotStatus(dockerfile("snapshot=20261120T000000Z"), day("2026-11-22")),
    { snapshot: "20261120T000000Z", ageDays: 2, stale: false },
  );
});

test("la mention `snapshot=` d'un commentaire n'est pas une date", () => {
  const text = "# changer `snapshot=` ci-dessous\n" + dockerfile("snapshot=20261120T000000Z");
  assert.equal(snapshotStatus(text, day("2026-11-22")).snapshot, "20261120T000000Z");
});

test("un instantané de plus de MAX_AGE_DAYS jours est signalé", () => {
  const status = snapshotStatus(
    dockerfile("snapshot=20261001T000000Z"),
    day("2026-11-22"),
  );
  assert.equal(status.stale, true);
  assert.ok(status.ageDays > MAX_AGE_DAYS);
});

test("un instantané de MAX_AGE_DAYS jours pile n'est pas encore signalé", () => {
  const status = snapshotStatus(
    dockerfile("snapshot=20261018T000000Z"),
    day("2026-11-22"),
  );
  assert.equal(status.ageDays, MAX_AGE_DAYS);
  assert.equal(status.stale, false);
});

test("date absente : erreur", () => {
  assert.throws(
    () => snapshotStatus(dockerfile("true"), day("2026-11-22")),
    /instantané/,
  );
});

test("date mal formée ou impossible : erreur", () => {
  for (const bad of ["snapshot=2026-11-20", "snapshot=20261340T000000Z"]) {
    assert.throws(
      () => snapshotStatus(dockerfile(bad), day("2026-11-22")),
      /instantané/,
    );
  }
});

test("plusieurs dates : erreur", () => {
  const text =
    dockerfile("snapshot=20261120T000000Z") +
    dockerfile("snapshot=20261101T000000Z");
  assert.throws(() => snapshotStatus(text, day("2026-11-22")), /instantané/);
});

test("date dans le futur : erreur", () => {
  assert.throws(
    () =>
      snapshotStatus(dockerfile("snapshot=20261201T000000Z"), day("2026-11-22")),
    /instantané/,
  );
});

test("lancé par un chemin qui traverse un lien symbolique, le contrôle s'exécute", async () => {
  const { execFileSync } = await import("node:child_process");
  const { mkdtempSync, symlinkSync, rmSync } = await import("node:fs");
  const os = await import("node:os");
  const path = await import("node:path");
  const { fileURLToPath } = await import("node:url");
  const scripts = path.dirname(fileURLToPath(import.meta.url));
  const dir = mkdtempSync(path.join(os.tmpdir(), "norelly-lien-"));
  try {
    const link = path.join(dir, "scripts");
    try {
      symlinkSync(scripts, link, "dir");
    } catch {
      return; // pas de lien symbolique possible (Windows sans droits)
    }
    const out = execFileSync(process.execPath, [path.join(link, "check-apt-snapshot.mjs")], {
      encoding: "utf8",
    });
    assert.match(out, /Instantané apt de l'e2e/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
