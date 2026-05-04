import { RoutingController } from "./controllers/routingController";
import { renderRoutingView } from "./views/routing";

type RouteRenderer = () => string;

export class Router {
  private routes: Record<string, RouteRenderer>;

  constructor() {
    this.routes = {
      "/dashboard": () => `
        <div class="space-y-6">
          <div>
            <h1 class="text-2xl font-semibold text-slate-100">Dashboard</h1>
            <p class="text-slate-400">Overview of your Edge Mesh</p>
          </div>
          <div class="grid gap-4 md:grid-cols-3">
            <div class="rounded-lg border border-slate-800 bg-slate-900 p-4">
              <div class="text-sm text-slate-400">Control Plane</div>
              <div class="mt-2 text-2xl text-cyan-400">Online</div>
            </div>
            <div class="rounded-lg border border-slate-800 bg-slate-900 p-4">
              <div class="text-sm text-slate-400">Runtime Targets</div>
              <div class="mt-2 text-2xl text-slate-100">Ready</div>
            </div>
            <div class="rounded-lg border border-slate-800 bg-slate-900 p-4">
              <div class="text-sm text-slate-400">GitOps State</div>
              <div class="mt-2 text-2xl text-emerald-400">Synced</div>
            </div>
          </div>
        </div>
      `,
      "/routing": () => renderRoutingView(),
      "/security": () => `
        <div>
          <h1 class="text-2xl font-semibold text-slate-100">Security & IAM</h1>
          <p class="text-slate-400">Control plane access and identity policy</p>
        </div>
      `,
    };
    window.addEventListener("hashchange", () => this.handleRoute());
  }

  public handleRoute() {
    const path = window.location.hash.replace("#", "") || "/dashboard";
    const renderer = this.routes[path] || this.routes["/dashboard"];
    const viewContainer = document.getElementById("route-view");

    if (viewContainer) {
      window.dispatchEvent(
        new CustomEvent("route-change", {
          detail: { path, renderer, container: viewContainer },
        }),
      );
    }
  }

  public initRoute(path: string) {
    if (path === "/routing") {
      RoutingController.init();
    }
  }
}
