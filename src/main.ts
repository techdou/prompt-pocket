import "./app.css";
import App from "./App.svelte";
import { mount } from "svelte";
import { installPreview } from "./preview";

if (import.meta.env.DEV && new URLSearchParams(location.search).has("preview") && !("__TAURI_INTERNALS__" in window)) {
  installPreview();
  document.title = "Prompt Pocket · Browser preview (sample data)";
}

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
