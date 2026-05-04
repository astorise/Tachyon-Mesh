# Design: AI Orchestration UI Component

## 1. The View Template (`src/views/aiOrchestration.ts`)
This template generates the dashboard for LLM deployments.

    export function renderAiOrchestrationView(): string {
        return `
        <div class="h-full flex flex-col gap-6">
            <div class="flex items-center justify-between">
                <div>
                    <h1 class="text-2xl font-bold text-slate-100">AI Orchestration</h1>
                    <p class="text-sm text-slate-400">Manage LLMs, Candle Engine settings, and LoRA Multiplexing.</p>
                </div>
                <button id="btn-deploy-ai" class="px-4 py-2 bg-fuchsia-500/10 text-fuchsia-400 border border-fuchsia-500/30 rounded shadow-[0_0_15px_rgba(217,70,239,0.2)] hover:bg-fuchsia-500/20 transition-all">
                    Deploy AI Config
                </button>
            </div>

            <div class="grid grid-cols-1 xl:grid-cols-3 gap-6 flex-grow">
                
                <div class="xl:col-span-2 flex flex-col gap-4">
                    <div class="flex justify-between items-center">
                        <h2 class="text-lg font-semibold text-slate-200">Deployed Base Models</h2>
                        <button class="text-sm text-fuchsia-400 hover:text-fuchsia-300 transition-colors">+ Add Model</button>
                    </div>

                    <div class="ai-card bg-slate-900 border border-slate-800 rounded-lg p-5 hover:border-fuchsia-500/50 transition-colors relative overflow-hidden">
                        <div class="absolute -top-10 -right-10 w-32 h-32 bg-fuchsia-500/10 blur-3xl rounded-full pointer-events-none"></div>
                        
                        <div class="flex justify-between items-start mb-4">
                            <div>
                                <h3 class="text-lg font-bold text-slate-100">llama-3-core</h3>
                                <p class="text-xs text-slate-500 font-mono">Asset: llama-3-8b-instruct.gguf</p>
                            </div>
                            <span class="px-2 py-1 bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 rounded text-xs">VRAM Pinned</span>
                        </div>

                        <div class="grid grid-cols-3 gap-4 mb-4 p-3 bg-slate-950 rounded border border-slate-800/50">
                            <div>
                                <label class="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">Backend</label>
                                <span class="text-sm text-slate-300">Auto (Metal)</span>
                            </div>
                            <div>
                                <label class="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">Quantization</label>
                                <span class="text-sm text-slate-300">Q4_K_M</span>
                            </div>
                            <div>
                                <label class="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">Context Window</label>
                                <span class="text-sm text-slate-300">8192 tokens</span>
                            </div>
                        </div>

                        <div class="flex items-center justify-between text-sm">
                            <span class="text-slate-400">Distribution: <span class="text-fuchsia-400">Tensor Parallelism</span></span>
                            <span class="text-slate-400">Multi-GPU: <span class="text-slate-200">Enabled</span></span>
                        </div>
                    </div>
                </div>

                <div class="xl:col-span-1 bg-slate-900 border border-slate-800 rounded-lg p-5 flex flex-col">
                    <h2 class="text-lg font-semibold text-slate-200 mb-4">LoRA Multiplexing</h2>
                    <p class="text-xs text-slate-400 mb-4">Dynamically route requests to specific adapters without reloading the base model.</p>
                    
                    <div class="flex-grow flex flex-col gap-3">
                        <div class="p-3 bg-slate-950 border border-slate-800 rounded lora-rule">
                            <div class="flex justify-between items-center mb-2">
                                <span class="text-sm font-medium text-slate-200">Legal Assistant</span>
                                <span class="text-[10px] bg-slate-800 text-slate-400 px-1.5 rounded">Adapter</span>
                            </div>
                            <div class="text-xs font-mono text-slate-500 bg-slate-900 p-1.5 rounded">
                                X-Tenant-Domain == "legal"
                            </div>
                        </div>

                        <div class="p-3 bg-slate-950 border border-slate-800 rounded lora-rule">
                            <div class="flex justify-between items-center mb-2">
                                <span class="text-sm font-medium text-slate-200">Code Copilot</span>
                                <span class="text-[10px] bg-slate-800 text-slate-400 px-1.5 rounded">Adapter</span>
                            </div>
                            <div class="text-xs font-mono text-slate-500 bg-slate-900 p-1.5 rounded">
                                X-Tenant-Domain == "engineering"
                            </div>
                        </div>
                    </div>
                    
                    <button class="mt-4 w-full py-2 bg-slate-800 hover:bg-slate-700 text-slate-300 text-sm rounded transition-colors border border-slate-700">
                        + Add Routing Rule
                    </button>
                </div>

            </div>
        </div>
        `;
    }

## 2. Vanilla JS Controller (`src/controllers/aiController.ts`)
This controller binds the actions and serializes the DOM state to the GIT-Ops JSON.

    export class AiOrchestrationController {
        static init() {
            // Animate cards on entry using GSAP
            if ((window as any).gsap) {
                const gsap = (window as any).gsap;
                gsap.from(".ai-card", { opacity: 0, y: 20, stagger: 0.1, duration: 0.4, ease: "power2.out" });
                gsap.from(".lora-rule", { opacity: 0, x: 20, stagger: 0.1, duration: 0.4, ease: "power2.out", delay: 0.2 });
            }

            const deployBtn = document.getElementById('btn-deploy-ai');
            if (!deployBtn) return;

            deployBtn.addEventListener('click', () => {
                if ((window as any).gsap) {
                    (window as any).gsap.to(deployBtn, { scale: 0.95, duration: 0.1, yoyo: true, repeat: 1 });
                }

                // Construct exact payload for Domain 10
                const payload = {
                    api_version: "ai.tachyon.io/v1alpha1",
                    kind: "AiOrchestration",
                    metadata: {
                        name: "global-inference-fleet",
                        environment: "production"
                    },
                    spec: {
                        model_deployments: [
                            {
                                name: "llama-3-core",
                                asset_ref: "llama-3-8b-instruct",
                                engine: {
                                    backend: "auto_detect",
                                    quantization: "q4_k_m",
                                    max_context_window: 8192,
                                    flash_attention: true
                                },
                                hardware_strategy: {
                                    multi_gpu: true,
                                    distribution_mode: "tensor_parallelism"
                                },
                                sharing_strategy: {
                                    mode: "lora_multiplexing",
                                    base_model_memory_lock: true,
                                    adapters: [
                                        {
                                            name: "legal-assistant-lora",
                                            asset_ref: "lora-legal-v1",
                                            routing_condition: { header_match: { "X-Tenant-Domain": "legal" } }
                                        }
                                    ]
                                }
                            }
                        ]
                    }
                };

                console.log("Pushing to System FaaS Config API:", JSON.stringify(payload, null, 2));
            });
        }
    }