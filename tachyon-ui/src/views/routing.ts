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
          <h2 class="text-lg font-semibold text-slate-200 mb-4">L4 Gateway</h2>
          <div class="flex flex-col gap-3">
            <label class="block text-xs text-slate-500">Gateway Name
              <input type="text" id="input-gw-name" value="public-https" class="mt-1 w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-cyan-500">
            </label>
            <label class="block text-xs text-slate-500">Protocol
              <select id="input-gw-protocol" class="mt-1 w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-cyan-500">
                <option value="HTTPS">HTTPS</option>
                <option value="HTTP">HTTP</option>
                <option value="TCP">TCP</option>
              </select>
            </label>
            <label class="block text-xs text-slate-500">Bind Address
              <input type="text" id="input-gw-bind" value="0.0.0.0:443" class="mt-1 w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500">
            </label>
          </div>
        </div>

        <div class="col-span-1 lg:col-span-2 bg-slate-900 rounded-lg border border-slate-800 p-4">
          <h2 class="text-lg font-semibold text-slate-200 mb-4">L7 Route Rule</h2>
          <div class="flex flex-col gap-3">
            <div class="grid grid-cols-2 gap-4">
              <label class="block text-xs text-slate-500">Route Name
                <input type="text" id="input-route-name" value="api-v1-routing" class="mt-1 w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-cyan-500">
              </label>
              <label class="block text-xs text-slate-500">Match Path (Prefix)
                <input type="text" id="input-route-path" value="/v1/inference" class="mt-1 w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-emerald-400 font-mono focus:outline-none focus:border-cyan-500">
              </label>
            </div>
            <label class="block text-xs text-slate-500">Target Workload (Asset Ref)
              <input type="text" id="input-route-target" value="ai-inference-wasm" class="mt-1 w-full bg-slate-950 border border-slate-800 rounded px-3 py-2 text-sm text-slate-200 focus:outline-none focus:border-cyan-500">
            </label>
          </div>
        </div>
      </div>
    </div>
  `;
}
