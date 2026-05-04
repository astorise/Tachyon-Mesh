import gsap from "gsap";

type TrafficConfiguration = {
  api_version: "routing.tachyon.io/v1alpha1";
  kind: "TrafficConfiguration";
  metadata: {
    name: string;
    environment: string;
  };
  spec: {
    gateways: Array<{
      name: string;
      protocol: string;
      bind_address: string;
    }>;
    routes: Array<{
      name: string;
      gateway_refs: string[];
      type: "HTTP";
      rules: Array<{
        match: {
          path: {
            prefix: string;
          };
        };
        target: string;
      }>;
    }>;
  };
};

export class RoutingController {
  static init() {
    const deployBtn = document.getElementById("btn-deploy-routing");
    if (!deployBtn) {
      return;
    }

    deployBtn.addEventListener("click", () => {
      gsap.to(deployBtn, { scale: 0.95, duration: 0.1, yoyo: true, repeat: 1 });
      const payload = this.buildPayload();
      console.log("Pushing to System FaaS Config API:", JSON.stringify(payload, null, 2));
      this.showToast("Configuration Applied Successfully");
    });
  }

  private static buildPayload(): TrafficConfiguration {
    const gatewayName = readText("[data-routing-gateway-name]", "public-https");
    const gatewayProtocol = readText("[data-routing-gateway-protocol]", "HTTPS");
    const bindAddress = readText("[data-routing-gateway-bind]", "0.0.0.0:443");
    const routeName = readText("[data-routing-route-name]", "api-v1-routing");
    const routePath = readText("[data-routing-route-path]", "/v1/inference");
    const routeTarget = readText("[data-routing-route-target]", "ai-inference-wasm");

    return {
      api_version: "routing.tachyon.io/v1alpha1",
      kind: "TrafficConfiguration",
      metadata: {
        name: "edge-main-routing",
        environment: "production",
      },
      spec: {
        gateways: [
          {
            name: gatewayName,
            protocol: gatewayProtocol,
            bind_address: bindAddress,
          },
        ],
        routes: [
          {
            name: routeName,
            gateway_refs: [gatewayName],
            type: "HTTP",
            rules: [
              {
                match: { path: { prefix: routePath } },
                target: routeTarget,
              },
            ],
          },
        ],
      },
    };
  }

  private static showToast(message: string) {
    const toast = document.createElement("div");
    toast.className =
      "fixed bottom-6 right-6 z-50 rounded border border-emerald-500/30 bg-slate-900 px-4 py-3 text-sm text-emerald-300 shadow-lg";
    toast.textContent = message;
    document.body.appendChild(toast);
    gsap.fromTo(toast, { opacity: 0, y: 12 }, { opacity: 1, y: 0, duration: 0.2 });
    gsap.to(toast, {
      opacity: 0,
      y: 12,
      delay: 2,
      duration: 0.2,
      onComplete: () => toast.remove(),
    });
  }
}

function readText(selector: string, fallback: string): string {
  return document.querySelector(selector)?.textContent?.trim() || fallback;
}
