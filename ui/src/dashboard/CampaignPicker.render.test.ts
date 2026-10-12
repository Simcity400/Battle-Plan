import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';

test('widget campaign choices show recording display names while preserving paths and layout labels', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'campaign-picker-fixture', enforce: 'pre',
      transform(_source, id) {
        if (id.replaceAll('\\', '/').endsWith('/components/Popover.tsx')) return `export const usePopover = () => ({ trigger: null, open: true, close() {} });
          export const Popover = ({ children }) => children;`;
      },
    }] });
  try {
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/dashboard/CampaignPicker.tsx') as typeof import('./CampaignPicker');
    const { saveRecordingNames, setRecordingName } = await vite.ssrLoadModule('/ui/src/dashboard/recordingNames.ts') as typeof import('./recordingNames.ts');
    const template = { roles: [{ id: 'primary', label: 'Original.battleplan' }], tracks: [{ id: 'main', roleId: 'primary' }] } as DashboardTemplate;
    const widget = { id: 'widget', campaign: 'tracks', trackIds: ['main'] } as WidgetSpec;
    const runtime = { campaigns: [{ id: 'primary', path: 'C:/Original.battleplan' }, { id: 'pending', path: 'C:/Pending.battleplan', loading: true }],
      defaultRoleId: 'primary', recordings: [{ path: 'C:/Archive.battleplan', name: 'Archive', folder: 'Past' }], assignCampaign: async () => {} } as unknown as WidgetRuntime;
    const before = structuredClone({ template, widget, campaigns: runtime.campaigns, recordings: runtime.recordings });
    const names = setRecordingName(setRecordingName(setRecordingName({}, 'c:\\Original.battleplan', 'Weekend campaign'),
      'C:/Archive.battleplan', 'Old favourite'), 'C:/Pending.battleplan', 'Next campaign');
    saveRecordingNames(names, { getItem: () => null, setItem() {} });
    const markup = () => renderToStaticMarkup(createElement(Picker, { template, widget, runtime }));
    assert.match(markup(), /aria-label="Widget campaign: Weekend campaign"/);
    assert.match(markup(), /Now Weekend campaign/);
    assert.match(markup(), />Weekend campaign<\/span>/);
    assert.match(markup(), /Loading Next campaign/);
    assert.match(markup(), /Old favourite<small>Past<\/small>/);
    assert.match(markup(), /data-tooltip="Weekend campaign · Original.battleplan"/);
    assert.match(markup(), /data-tooltip="Old favourite · Archive.battleplan"/);
    assert.deepEqual({ template, widget, campaigns: runtime.campaigns, recordings: runtime.recordings }, before);
    saveRecordingNames({}, { getItem: () => null, setItem() {} });
    assert.match(markup(), /aria-label="Widget campaign: Original"/);
    assert.match(markup(), /Archive<small>Past<\/small>/);
    assert.doesNotMatch(markup(), /Weekend campaign|Old favourite|Next campaign/);
  } finally { await vite.close(); }
});
