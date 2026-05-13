import gsap from "gsap";

import { connectionStore, type ConnectionState } from "../stores/connectionStore";

const statusColors: Record<ConnectionState, string> = {
  connected: "#22c55e",
  disconnected: "#ef4444",
  reconnecting: "#f59e0b",
};

export function mountNetworkStatus(host: HTMLElement): void {
  const root = document.createElement("div");
  root.className = "network-status inline-flex items-center gap-2 rounded border border-slate-700 px-3 py-1 text-xs";
  root.setAttribute("role", "status");
  root.setAttribute("aria-live", "polite");
  root.setAttribute("aria-atomic", "true");
  root.setAttribute("aria-label", "Connection status");

  const dot = document.createElement("span");
  dot.className = "network-status-dot h-2.5 w-2.5 rounded-full bg-green-500";

  const label = document.createElement("span");
  label.className = "network-status-label font-medium text-slate-200";
  label.textContent = "Connected";

  const retryBtn = document.createElement("button");
  retryBtn.type = "button";
  retryBtn.className =
    "network-retry-btn hidden rounded border border-red-500/50 bg-red-500/10 px-2 py-0.5 text-[10px] font-medium text-red-300 hover:bg-red-500/20";
  retryBtn.textContent = "Retry";
  retryBtn.addEventListener("click", () => {
    connectionStore.getState().manualRetry();
  });

  root.appendChild(dot);
  root.appendChild(label);
  root.appendChild(retryBtn);
  host.appendChild(root);

  const render = (status: ConnectionState, attempt: number, maxAttempts: number) => {
    const isTerminal = status === "disconnected" && maxAttempts > 0 && attempt >= maxAttempts;

    if (status === "reconnecting") {
      label.textContent = `Reconnecting (${attempt}/${maxAttempts})`;
    } else if (isTerminal) {
      label.textContent = "Cluster unreachable";
    } else if (status === "disconnected") {
      label.textContent = "Offline";
    } else {
      label.textContent = "Connected";
    }

    retryBtn.classList.toggle("hidden", status !== "disconnected");

    gsap.to(dot, {
      backgroundColor: statusColors[status],
      scale: status === "reconnecting" ? 1.15 : 1,
      duration: 0.25,
      ease: "power2.out",
    });
  };

  const { status, attempt, maxAttempts } = connectionStore.getState();
  render(status, attempt, maxAttempts);
  connectionStore.subscribe((state) => render(state.status, state.attempt, state.maxAttempts));
}
