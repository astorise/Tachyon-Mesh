# Design: Routing & Gateways UI Component

## 1. The View Template (`src/views/routing.ts`)
We encapsulate the HTML string and its associated logic in a module to keep `router.ts` clean.

    export function renderRoutingView(): string {
        return `
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
                
                <div class="col-span-1 bg-slate-900 rounded-lg border border-slate-800 p-4">
                    <div class="flex justify-between items-center mb-4">
                        <h2 class="text-lg font-semibold text-slate-200">Gateways (L4)</h2>
                        <button class="text-slate-400 hover:text-emerald-400">+</button>
                    </div>
                    <div class="p-3 bg-slate-950 border border-slate-800 rounded group hover:border-slate-700 transition-colors">
                        <div class="flex justify-between">
                            <span class="font-mono text-sm text-cyan-400">public-https</span>
                            <span class="text-xs bg-slate-800 text-slate-300 px-2 py-0.5 rounded">HTTPS</span>
                        </div>
                        <p class="text-xs text-slate-500 mt-2">Bind: 0.0.0.0:443</p>
                    </div>
                </div>

                <div class="col-span-1 lg:col-span-2 bg-slate-900 rounded-lg border border-slate-800 p-4">
                    <div class="flex justify-between items-center mb-4">
                        <h2 class="text-lg font-semibold text-slate-200">Traffic Routes (L7)</h2>
                        <button class="px-3 py-1 text-sm bg-slate-800 hover:bg-slate-700 text-slate-200 rounded transition-colors">+ New Route</button>
                    </div>
                    <table class="w-full text-left text-sm text-slate-400">
                        <thead class="bg-slate-950/50 text-slate-500">
                            <tr>
                                <th class="px-4 py-2 font-medium rounded-tl">Name</th>
                                <th class="px-4 py-2 font-medium">Gateway</th>
                                <th class="px-4 py-2 font-medium">Match Path</th>
                                <th class="px-4 py-2 font-medium rounded-tr">Target</th>
                            </tr>
                        </thead>
                        <tbody class="divide-y divide-slate-800/50">
                            <tr class="hover:bg-slate-800/20 transition-colors">
                                <td class="px-4 py-3 font-medium text-slate-200">api-v1-routing</td>
                                <td class="px-4 py-3 font-mono text-xs">public-https</td>
                                <td class="px-4 py-3 text-emerald-400">/v1/inference</td>
                                <td class="px-4 py-3">ai-inference-wasm</td>
                            </tr>
                        </tbody>
                    </table>
                </div>
            </div>
        </div>
        `;
    }

## 2. Vanilla JS Data Binding (`src/controllers/routingController.ts`)
This controller is executed right after the router injects the HTML. It gathers DOM data and shapes it into our specific JSON format.

    export class RoutingController {
        static init() {
            const deployBtn = document.getElementById('btn-deploy-routing');
            if (!deployBtn) return;

            deployBtn.addEventListener('click', () => {
                // Animate button click with GSAP
                gsap.to(deployBtn, { scale: 0.95, duration: 0.1, yoyo: true, repeat: 1 });

                // Construct the exact JSON schema defined in Domain 1 (Traffic Management)
                const payload = {
                    api_version: "routing.tachyon.io/v1alpha1",
                    kind: "TrafficConfiguration",
                    metadata: {
                        name: "edge-main-routing",
                        environment: "production"
                    },
                    spec: {
                        gateways: [
                            // In a full implementation, read these from a <form>
                            { name: "public-https", protocol: "HTTPS", bind_address: "0.0.0.0:443" }
                        ],
                        routes: [
                            {
                                name: "api-v1-routing",
                                gateway_refs: ["public-https"],
                                type: "HTTP",
                                rules: [{ match: { path: { prefix: "/v1/inference" } } }]
                            }
                        ]
                    }
                };

                // Dispatch to the MCP/Tauri backend (Mocked here)
                console.log("Pushing to System FaaS Config API:", JSON.stringify(payload, null, 2));
                
                // Show success notification (Toast)
                this.showToast("Configuration Applied Successfully");
            });
        }

        static showToast(msg: string) {
            // Implementation of a simple GSAP animated toast notification
        }
    }