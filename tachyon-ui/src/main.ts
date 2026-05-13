import { listen } from "@tauri-apps/api/event";

import "./style.css";
import "./components/iam/TachyonIAM";
import "./components/layout/TachyonAppShell";
import { connectionStore } from "./stores/connectionStore";

document.addEventListener("DOMContentLoaded", () => {
  connectionStore.getState().setStatus("disconnected");

  void listen("mesh-disconnect", () => {
    document.getElementById("auth-layer")?.classList.remove("hidden");
    document.querySelector("tachyon-iam")?.classList.remove("hidden");
    document.querySelector("tachyon-app-shell")?.classList.add("hidden");
    connectionStore.getState().setStatus("disconnected");
  });

  void listen("mesh-connect", () => {
    connectionStore.getState().resetRetry();
    connectionStore.getState().setStatus("connected");
  });
});
