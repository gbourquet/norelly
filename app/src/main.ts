import { mount } from "svelte";
import "./styles/tokens.css";
import App from "./App.svelte";

const target = document.getElementById("app");
if (!target) throw new Error("élément #app introuvable");

export default mount(App, { target });
