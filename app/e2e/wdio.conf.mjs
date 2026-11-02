// Configuration WebdriverIO des tests end-to-end (ADR-007), exécutée dans le
// conteneur e2e (voir run.sh) : tauri-driver pilote l'application via
// WebKitWebDriver.
import { spawn } from "node:child_process";
import net from "node:net";

const application = process.env.NORELLY_APP_BINARY;
if (!application) throw new Error("NORELLY_APP_BINARY non défini (lancer via pnpm -C app e2e)");

let tauriDriver;

/** Attend que tauri-driver accepte les connexions. */
async function waitForPort(port, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const ok = await new Promise((resolve) => {
      const socket = net.connect(port, "127.0.0.1");
      socket.once("connect", () => {
        socket.destroy();
        resolve(true);
      });
      socket.once("error", () => resolve(false));
    });
    if (ok) return;
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`tauri-driver ne répond pas sur le port ${port}`);
}

export const config = {
  runner: "local",
  specs: ["./specs/**/*.e2e.mjs"],
  maxInstances: 1,
  hostname: "127.0.0.1",
  port: 4444,
  capabilities: [{ maxInstances: 1, "tauri:options": { application } }],
  logLevel: "warn",
  bail: 0,
  waitforTimeout: 20000,
  connectionRetryTimeout: 120000,
  connectionRetryCount: 1,
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { ui: "bdd", timeout: 240000 },

  beforeSession: async () => {
    tauriDriver = spawn("tauri-driver", [], { stdio: ["ignore", "inherit", "inherit"] });
    await waitForPort(4444, 20000);
  },

  afterSession: () => {
    tauriDriver?.kill();
  },
};
