# Technical Specification: UI Feature Parity

## 1. VRAM Observability (`src/components/domains/TachyonHardwarePanel.ts`)
Extend the hardware panel to consume the new VRAM metrics from the core telemetry stream.
Implement a visual indicator for GPU pressure.

```typescript
// Example HTML structure to inject
const vramWidget = `
  <div class="vram-widget mt-6">
    <h3 class="text-lg font-semibold">GPU VRAM Allocation</h3>
    ${gpus.map(gpu => `
      <div class="mb-4">
        <div class="flex justify-between text-sm">
          <span>${gpu.name} (${gpu.id})</span>
          <span>${gpu.used_vram_mb}MB / ${gpu.total_vram_mb}MB</span>
        </div>
        <div class="w-full bg-gray-200 rounded-full h-2.5 dark:bg-gray-700 mt-1">
          <div class="bg-purple-600 h-2.5 rounded-full" style="width: ${(gpu.used_vram_mb / gpu.total_vram_mb) * 100}%"></div>
        </div>
      </div>
    `).join('')}
  </div>
`;
```

## 2. KV Explorer (`src/components/domains/TachyonStoragePanel.ts`)
Create a new panel (or tab within the Storage domain) dedicated to KV-Partition V2.
It must fetch namespaces and display a paginated or scrollable table of keys and their serialized values.

```typescript
// Must include actions to delete or inspect keys
interface KvEntry {
  namespace: string;
  key: string;
  value_preview: string;
  size_bytes: number;
  last_modified: string;
}
// Render as a data grid with an "Inspect" modal for large JSON values.
```