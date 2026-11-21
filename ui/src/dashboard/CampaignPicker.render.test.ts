import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';

test('widget campaign choices show recordings by name without extension while preserving paths and layout labels', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'campaign-picker-fixture', enforce: 'pre',
      transform(_source, id) {
        if (id.replaceAll('\\', '/').endsWith('/components/Popover.tsx')) return `export const usePopover = () => ({ trigger: null, open: true, close() {} });
          export const Popover = ({ children }) => children;`;
      },
    }] });
  try {
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/dashboard/CampaignPicker.tsx') as typeof import('./CampaignPicker');
    const template = { roles: [{ id: 'primary', label: 'Original.battleplan' }], tracks: [{ id: 'main', roleId: 'primary' }] } as DashboardTemplate;
    const widget = { id: 'widget', campaign: 'tracks', trackIds: ['main'] } as WidgetSpec;
    const runtime = { campaigns: [{ id: 'primary', path: 'C:/Germany 1936–1941.battleplan' }, { id: 'pending', path: 'C:/Italy 1936.battleplan', loading: true }],
      defaultRoleId: 'primary', recordings: [{ path: 'C:/Japan 1936–1945 (Multiplayer).battleplan', name: 'Japan 1936–1945 (Multiplayer)', folder: 'Past' }],
      assignCampaign: async () => {} } as unknown as WidgetRuntime;
    const before = structuredClone({ template, widget, campaigns: runtime.campaigns, recordings: runtime.recordings });
    const markup = () => renderToStaticMarkup(createElement(Picker, { template, widget, runtime }));
    assert.match(markup(), /aria-label="Widget campaign: Germany 1936–1941"/);
    assert.match(markup(), /Now Germany 1936–1941/);
    assert.match(markup(), /data-overflow-label="true">Germany 1936–1941<\/span>/);
    assert.match(markup(), /Loading Italy 1936/);
    assert.match(markup(), /Japan 1936–1945 \(Multiplayer\)<small>Past<\/small>/);
    assert.match(markup(), /data-tooltip-overflow="Germany 1936–1941"/, 'a clipped name shows in full on hover');
    assert.match(markup(), /data-tooltip-overflow="Japan 1936–1945 \(Multiplayer\)"/);
    assert.doesNotMatch(markup(), /\.battleplan/, 'no file extension is shown');
    assert.deepEqual({ template, widget, campaigns: runtime.campaigns, recordings: runtime.recordings }, before);
  } finally { await vite.close(); }
});
