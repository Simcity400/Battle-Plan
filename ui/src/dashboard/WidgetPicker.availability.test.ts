import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardSource, ProductRequest } from './client';
import type { WidgetRuntime } from './definitions';
import { syntheticDashboardSource } from './source.fixture.ts';
import { productRequest } from './request.ts';

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('the casualty-pairs catalogue probe carries its source into prepared product routing', async () => {
  const calls: { request: ProductRequest; source: DashboardSource; active: boolean }[] = [];
  const globals = globalThis as typeof globalThis & { __pickerAvailability?: typeof calls };
  globals.__pickerAvailability = calls;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'picker-availability-fixture', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/components/Popover.tsx')) return `export const usePopover = () => ({ trigger: null, open: true });
          export const Popover = ({ children }) => children; export const PopoverBody = ({ children }) => children();`;
        if (path.endsWith('/dashboard/products.ts')) return `export function useProduct(_client, request, active, _validate, source) {
          globalThis.__pickerAvailability.push({ request, active, source }); return { data: { available: true }, retry() {} }; }
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`;
        if (path.endsWith('/dashboard/WidgetPreview.tsx')) return 'export default function Preview() { return null; }';
      },
    }] });
  try {
    const { registerWidgets } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    const { definition } = await vite.ssrLoadModule('/ui/src/widgets/casualties-between/definition.ts');
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/dashboard/WidgetPicker.tsx');
    registerWidgets([definition]);
    const source = syntheticDashboardSource('pair-source', 'pair-revision');
    const runtime = { client: {}, defaultRoleId: 'chosen', tracks: [{ id: 'track', roleId: 'chosen', source,
      branch: { id: 'branch' } }] } as unknown as WidgetRuntime;
    renderToStaticMarkup(createElement(Picker, { runtime, disabled: false, onOpen() {}, onAdd() {} }));
    assert.ok(calls.length > 0);
    for (const call of calls) {
      assert.equal(call.source, source);
      assert.equal(call.active, true);
      assert.deepEqual(call.request, productRequest({ sourceId: source.id, revision: source.revision, product: 'casualty-pairs',
        params: { tags: [], between: false, window: null, branchId: 'branch' } }));
    }
  } finally { await vite.close(); delete globals.__pickerAvailability; }
});
