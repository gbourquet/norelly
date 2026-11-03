import { mount } from "svelte";
import "./styles/tokens.css";
import App from "./App.svelte";
import { applyLanguage } from "./lib/i18n";
import { api } from "./lib/ipc/api";

const target = document.getElementById("app");
if (!target) throw new Error("élément #app introuvable");

/** Langue des paramètres (celle du système par défaut, ou si les
 * paramètres sont illisibles), appliquée avant le premier rendu
 * (ADR-011). */
async function start(root: HTMLElement): Promise<ReturnType<typeof mount>> {
  const language = await api
    .getSettings()
    .then((s) => s.language)
    .catch(() => "system" as const);
  applyLanguage(language, navigator);
  return mount(App, { target: root });
}

export default start(target);
