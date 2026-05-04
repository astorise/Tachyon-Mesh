export function renderAiOrchestrationView(): string {
  return `
    <div class="flex h-full flex-col gap-6">
      <div class="flex items-center justify-between gap-4">
        <div>
          <h1 class="text-2xl font-bold text-slate-100">AI Orchestration</h1>
          <p class="text-sm text-slate-400">Manage LLMs, Candle Engine settings, and LoRA Multiplexing.</p>
        </div>
        <button id="btn-deploy-ai" class="rounded border border-fuchsia-500/30 bg-fuchsia-500/10 px-4 py-2 text-fuchsia-400 shadow-[0_0_15px_rgba(217,70,239,0.2)] transition-all hover:bg-fuchsia-500/20">
          Deploy AI Config
        </button>
      </div>

      <div class="grid flex-grow grid-cols-1 gap-6 xl:grid-cols-3">
        <div class="flex flex-col gap-4 xl:col-span-2">
          <div class="flex items-center justify-between">
            <h2 class="text-lg font-semibold text-slate-200">Deployed Base Models</h2>
            <button class="text-sm text-fuchsia-400 transition-colors hover:text-fuchsia-300">+ Add Model</button>
          </div>

          <div class="ai-card relative overflow-hidden rounded-lg border border-slate-800 bg-slate-900 p-5 transition-colors hover:border-fuchsia-500/50">
            <div class="mb-4 flex items-start justify-between">
              <div>
                <h3 class="text-lg font-bold text-slate-100" data-ai-model-name>llama-3-core</h3>
                <p class="font-mono text-xs text-slate-500">Asset: <span data-ai-model-asset>llama-3-8b-instruct.gguf</span></p>
              </div>
              <span class="rounded border border-emerald-500/20 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-400">VRAM Pinned</span>
            </div>

            <div class="mb-4 grid grid-cols-3 gap-4 rounded border border-slate-800/50 bg-slate-950 p-3">
              <div>
                <label class="mb-1 block text-[10px] uppercase tracking-wider text-slate-500">Backend</label>
                <span class="text-sm text-slate-300" data-ai-model-backend>Auto (Metal)</span>
              </div>
              <div>
                <label class="mb-1 block text-[10px] uppercase tracking-wider text-slate-500">Quantization</label>
                <span class="text-sm text-slate-300" data-ai-model-quantization>Q4_K_M</span>
              </div>
              <div>
                <label class="mb-1 block text-[10px] uppercase tracking-wider text-slate-500">Context Window</label>
                <span class="text-sm text-slate-300" data-ai-model-context>8192 tokens</span>
              </div>
            </div>

            <div class="flex items-center justify-between text-sm">
              <span class="text-slate-400">Distribution: <span class="text-fuchsia-400">Tensor Parallelism</span></span>
              <span class="text-slate-400">Multi-GPU: <span class="text-slate-200">Enabled</span></span>
            </div>
          </div>
        </div>

        <div class="flex flex-col rounded-lg border border-slate-800 bg-slate-900 p-5 xl:col-span-1">
          <h2 class="mb-4 text-lg font-semibold text-slate-200">LoRA Multiplexing</h2>
          <p class="mb-4 text-xs text-slate-400">Dynamically route requests to specific adapters without reloading the base model.</p>

          <div class="flex flex-grow flex-col gap-3">
            <div class="lora-rule rounded border border-slate-800 bg-slate-950 p-3">
              <div class="mb-2 flex items-center justify-between">
                <span class="text-sm font-medium text-slate-200" data-ai-lora-name>Legal Assistant</span>
                <span class="rounded bg-slate-800 px-1.5 text-[10px] text-slate-400">Adapter</span>
              </div>
              <div class="rounded bg-slate-900 p-1.5 font-mono text-xs text-slate-500" data-ai-lora-condition>
                X-Tenant-Domain == "legal"
              </div>
            </div>

            <div class="lora-rule rounded border border-slate-800 bg-slate-950 p-3">
              <div class="mb-2 flex items-center justify-between">
                <span class="text-sm font-medium text-slate-200">Code Copilot</span>
                <span class="rounded bg-slate-800 px-1.5 text-[10px] text-slate-400">Adapter</span>
              </div>
              <div class="rounded bg-slate-900 p-1.5 font-mono text-xs text-slate-500">
                X-Tenant-Domain == "engineering"
              </div>
            </div>
          </div>

          <button class="mt-4 w-full rounded border border-slate-700 bg-slate-800 py-2 text-sm text-slate-300 transition-colors hover:bg-slate-700">
            + Add Routing Rule
          </button>
        </div>
      </div>
    </div>
  `;
}
