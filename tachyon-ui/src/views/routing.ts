export function renderRoutingView(): string {
  return `
    <div class="flex h-full flex-col gap-6">
      <div class="flex items-center justify-between gap-4">
        <div>
          <h1 class="text-2xl font-bold text-slate-100">Routing & Gateways</h1>
          <p class="text-sm text-slate-400">Configure L4 listeners and L7 traffic splitting.</p>
        </div>
        <button id="btn-deploy-routing" class="rounded border border-cyan-500/30 bg-cyan-500/10 px-4 py-2 text-cyan-400 shadow-[0_0_15px_rgba(34,211,238,0.2)] transition-all hover:bg-cyan-500/20">
          Deploy Configuration
        </button>
      </div>

      <div class="grid flex-grow grid-cols-1 gap-6 lg:grid-cols-3">
        <div class="col-span-1 rounded-lg border border-slate-800 bg-slate-900 p-4">
          <div class="mb-4 flex items-center justify-between">
            <h2 class="text-lg font-semibold text-slate-200">Gateways (L4)</h2>
            <button class="text-slate-400 hover:text-emerald-400" aria-label="Add gateway">+</button>
          </div>
          <div class="group rounded border border-slate-800 bg-slate-950 p-3 transition-colors hover:border-slate-700">
            <div class="flex justify-between">
              <span class="font-mono text-sm text-cyan-400" data-routing-gateway-name>public-https</span>
              <span class="rounded bg-slate-800 px-2 py-0.5 text-xs text-slate-300" data-routing-gateway-protocol>HTTPS</span>
            </div>
            <p class="mt-2 text-xs text-slate-500">Bind: <span data-routing-gateway-bind>0.0.0.0:443</span></p>
          </div>
        </div>

        <div class="col-span-1 rounded-lg border border-slate-800 bg-slate-900 p-4 lg:col-span-2">
          <div class="mb-4 flex items-center justify-between">
            <h2 class="text-lg font-semibold text-slate-200">Traffic Routes (L7)</h2>
            <button class="rounded bg-slate-800 px-3 py-1 text-sm text-slate-200 transition-colors hover:bg-slate-700">+ New Route</button>
          </div>
          <table class="w-full text-left text-sm text-slate-400">
            <thead class="bg-slate-950/50 text-slate-500">
              <tr>
                <th class="rounded-tl px-4 py-2 font-medium">Name</th>
                <th class="px-4 py-2 font-medium">Gateway</th>
                <th class="px-4 py-2 font-medium">Match Path</th>
                <th class="rounded-tr px-4 py-2 font-medium">Target</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-slate-800/50">
              <tr class="transition-colors hover:bg-slate-800/20">
                <td class="px-4 py-3 font-medium text-slate-200" data-routing-route-name>api-v1-routing</td>
                <td class="px-4 py-3 font-mono text-xs" data-routing-route-gateway>public-https</td>
                <td class="px-4 py-3 text-emerald-400" data-routing-route-path>/v1/inference</td>
                <td class="px-4 py-3" data-routing-route-target>ai-inference-wasm</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>
  `;
}
