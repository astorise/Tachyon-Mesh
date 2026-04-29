import gsap from "gsap";

import { connectionStore, type ConnectionState } from "../stores/connectionStore";

const statusCopy: Record<ConnectionState, string> = {
  connected: "Connected",
  disconnected: "Offline",
  reconnecting: "Reconnecting",
};

const statusColors: Record<ConnectionState, string> = {
  connected: "#22c55e",
  disconnected: "#ef4444",
  reconnecting: "#f59e0b",
};

export function mountNetworkStatus(host: HTMLElement): void {
  const root = document.createElement("div");
  root.className = "network-status inline-flex items-center gap-2 rounded border border-slate-700 px-3 py-1 text-xs";
  root.innerHTML = `
    <span class="network-status-dot h-2.5 w-2.5 rounded-full bg-green-500"></span>
    <span class="network-status-label font-medium text-slate-200">Connected</span>
  `;
  host.appendChild(root);

  const dot = root.querySelector<HTMLElement>(".network-status-dot");
  const label = root.querySelector<HTMLElement>(".network-status-label");

  const render = (status: ConnectionState, retryCount: number) => {
    if (!dot || !label) {
      return;
    }
    label.textContent = status === "reconnecting" ? `${statusCopy[status]} ${retryCount}` : statusCopy[status];
    gsap.to(dot, {
      backgroundColor: statusColors[status],
      scale: status === "reconnecting" ? 1.15 : 1,
      duration: 0.25,
      ease: "power2.out",
    });
  };

  render(connectionStore.getState().status, connectionStore.getState().retryCount);
  connectionStore.subscribe((state) => render(state.status, state.retryCount));
}
