import { resilientInvoke as invoke } from "../utils/network";

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
};

type RoutingPayload = {
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
  static mount(): void {
    document.getElementById("btn-deploy-routing")?.addEventListener("click", () => {
      void RoutingController.deploy();
    });
  }

  private static async deploy(): Promise<void> {
    const response = await invoke<ApplyConfigurationResponse>("apply_configuration", {
      domain: "config-routing",
      payload: RoutingController.buildPayload(),
    });

    const level = response.success ? "info" : "error";
    console[level](`[routing] ${response.message}`);
  }

  private static buildPayload(): RoutingPayload {
    const getValue = (id: string, fallback: string): string => {
      const el = document.getElementById(id) as HTMLInputElement | HTMLSelectElement | null;
      const value = el?.value.trim();
      return value ? value : fallback;
    };

    const gatewayName = getValue("input-gw-name", "default-gw");
    const gatewayProtocol = getValue("input-gw-protocol", "HTTPS");
    const bindAddress = getValue("input-gw-bind", "0.0.0.0:443");
    const routeName = getValue("input-route-name", "default-route");
    const routePath = getValue("input-route-path", "/");
    const routeTarget = getValue("input-route-target", "default-target");

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
}
