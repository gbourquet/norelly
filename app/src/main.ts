import { mount } from "svelte";
import "./styles/tokens.css";
import App from "./App.svelte";
import { applySystemLocale } from "./lib/i18n";

// Langue choisie avant le premier rendu (ADR-011).
applySystemLocale(navigator);

const target = document.getElementById("app");
if (!target) throw new Error("élément #app introuvable");

export default mount(App, { target });
