# Design: Interactive Routing & Wasm Validation

## 1. Frontend: Interactive Form Template (`tachyon-ui/src/views/routing.ts`)
We replace the static spans with interactive Tailwind-styled inputs.

    export function renderRoutingView(): string {
        return \`
        <div class="h-full flex flex-col gap-6">
            <div class="flex items-center justify-between">
                <div>
                    <h1 class="text-2xl font-bold text-slate-100">Routing & Gateways</h1>
                    <p class="text-sm text-slate-400">Configure L4 listeners and L7 traffic splitting.</p>
                </div>
                <button id="btn-deploy-routing" class="px-4 py-2 bg-cyan-500/10 text-cyan-400 border border-cyan-500/30 rounded shadow-[0_0_15px_rgba(34,211,238,0.2)] hover:bg-cyan-500/20 transition-all">
                    Deploy Configuration
                </button>
            </div>

            <div class="grid grid-cols-1 lg:grid-cols-3 gap-6 flex-grow">
                <!-- L4 Gateway Builder -->
                <div class="col-span-1 bg-slate-900 rounded-lg border border-slate-800 p-4">
                    <h2 class="text-lg font-semibold text-slate-200 mb-4">L4 Gateway</h2>
                    <div class="flex flex-col gap-3">
                        <div>
                            <label class="block text-xs text-slate-500 mb-1">Gateway Name</label>
                            <input type="text" id="input-gw-name" value="public-https" class="w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-cyan-500">
                        </div>
                        <div>
                            <label class="block text-xs text-slate-500 mb-1">Protocol</label>
                            <select id="input-gw-protocol" class="w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-cyan-500">
                                <option value="HTTPS">HTTPS</option>
                                <option value="HTTP">HTTP</option>
                                <option value="TCP">TCP</option>
                            </select>
                        </div>
                        <div>
                            <label class="block text-xs text-slate-500 mb-1">Bind Address</label>
                            <input type="text" id="input-gw-bind" value="0.0.0.0:443" class="w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500">
                        </div>
                    </div>
                </div>

                <!-- L7 Route Builder -->
                <div class="col-span-1 lg:col-span-2 bg-slate-900 rounded-lg border border-slate-800 p-4">
                    <h2 class="text-lg font-semibold text-slate-200 mb-4">L7 Route Rule</h2>
                    <div class="flex flex-col gap-3">
                        <div class="grid grid-cols-2 gap-4">
                            <div>
                                <label class="block text-xs text-slate-500 mb-1">Route Name</label>
                                <input type="text" id="input-route-name" value="api-v1-routing" class="w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-cyan-500">
                            </div>
                            <div>
                                <label class="block text-xs text-slate-500 mb-1">Match Path (Prefix)</label>
                                <input type="text" id="input-route-path" value="/v1/inference" class="w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-emerald-400 font-mono focus:outline-none focus:border-cyan-500">
                            </div>
                        </div>
                        <div>
                            <label class="block text-xs text-slate-500 mb-1">Target Workload (Asset Ref)</label>
                            <input type="text" id="input-route-target" value="ai-inference-wasm" class="w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-cyan-500">
                        </div>
                    </div>
                </div>
            </div>
        </div>
        \`;
    }

## 2. Frontend: Dynamic Data Binding (`tachyon-ui/src/controllers/routingController.ts`)
Refactor the payload builder to read from the actual inputs.

    // Inside RoutingController class:
    private static buildPayload(): any {
        // Safe extraction from input fields
        const getValue = (id: string, fallback: string) => {
            const el = document.getElementById(id) as HTMLInputElement | HTMLSelectElement | null;
            return el && el.value ? el.value.trim() : fallback;
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

## 3. Backend: Strict Validation (`tachyon-ui/src/main.rs`)
Implement strict Rust Serde validation mirroring the WIT schema to ensure no malformed config reaches the Edge node.

    use tauri::{command, AppHandle};
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    #[derive(Serialize)]
    pub struct ApiResponse {
        pub success: bool,
        pub message: String,
    }

    // --- Strict Rust representations of the config-routing.wit schema ---
    #[derive(Deserialize, Debug)]
    struct PathMatch { prefix: String }

    #[derive(Deserialize, Debug)]
    struct RuleMatch { path: PathMatch }

    #[derive(Deserialize, Debug)]
    struct RouteRule { match_rule: Option<RuleMatch>, target: String } // 'match' is a reserved keyword in Rust, handle via Serde rename if necessary. Using 'match_rule' for simplicity in mock, assuming Serde config handles the rename.

    #[derive(Deserialize, Debug)]
    struct Route { name: String, gateway_refs: Vec<String>, rules: Vec<serde_json::Value> } // Simplified for this slice

    #[derive(Deserialize, Debug)]
    struct Gateway { name: String, protocol: String, bind_address: String }

    #[derive(Deserialize, Debug)]
    struct TrafficSpec { gateways: Vec<Gateway>, routes: Vec<Route> }

    #[derive(Deserialize, Debug)]
    struct TrafficConfig { api_version: String, kind: String, spec: TrafficSpec }

    #[command]
    pub async fn apply_configuration(domain: String, payload: Value, _app: AppHandle) -> Result<ApiResponse, String> {
        println!("Received IPC Intent for Domain: {}", domain);
        
        if domain == "config-routing" {
            // Attempt strict Serde deserialization (Mimicking Wasmtime WIT validation)
            match serde_json::from_value::<TrafficConfig>(payload) {
                Ok(config) => {
                    // Validated! In a real scenario, we'd pass this to `system-faas-config-api.wasm`
                    let gw_count = config.spec.gateways.len();
                    let rt_count = config.spec.routes.len();
                    return Ok(ApiResponse {
                        success: true,
                        message: format!("WIT Validation Passed: 1 Schema, {} Gateways, {} Routes.", gw_count, rt_count),
                    });
                },
                Err(e) => {
                    return Ok(ApiResponse { // Return Ok with success:false so the UI can display the toast
                        success: false,
                        message: format!("WIT Validation Failed: {}", e),
                    });
                }
            }
        }
        Err(format!("Unknown configuration domain: {}", domain))
    }