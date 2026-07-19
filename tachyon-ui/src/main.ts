import { listen } from "@tauri-apps/api/event";

import "./style.css";
import "./components/iam/TachyonIAM";
import "./components/layout/TachyonAppShell";
import { connectionStore } from "./stores/connectionStore";
import { installGlobalErrorLogging, logUiError } from "./utils/appLogger";

installGlobalErrorLogging();

document.addEventListener("DOMContentLoaded", () => {
  connectionStore.getState().setStatus("disconnected");

  void listen("mesh-disconnect", () => {
    document.getElementById("auth-layer")?.classList.remove("hidden");
    document.querySelector("tachyon-iam")?.classList.remove("hidden");
    document.querySelector("tachyon-app-shell")?.classList.add("hidden");
    connectionStore.getState().setStatus("disconnected");
  }).catch((error) => logUiError("event.mesh-disconnect.listen", error));

  void listen("mesh-connect", () => {
    connectionStore.getState().resetRetry();
    connectionStore.getState().setStatus("connected");
  }).catch((error) => logUiError("event.mesh-connect.listen", error));
});
