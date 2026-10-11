import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardTemplate, BoundTrack, WidgetSpec } from '../model';
import type { WidgetRuntime } from '../definitions';
import type { WidgetSettings } from './WidgetSettings';

test('faction colour settings follow the chart campaign, including default changes and unloaded sources', async () => {
  const replacements: Record<string, string> = {
    '/dashboard/FactionColours.tsx': `import { createElement } from 'react';
      export const FactionColoursEditor = ({ source }) => createElement('div', { 'data-edited-campaign': source.id });`,
    '/dashboard/CampaignPicker.tsx': 'export default () => null;',
    '/dashboard/settings/CountriesSection.tsx': 'export const CountriesSection = () => null;',
    '/dashboard/settings/OptionFields.tsx': 'export const OptionFields = () => null;',
    '/dashboard/settings/TimeSection.tsx': 'export const TimeSection = () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'faction-settings-campaign-fixture', enforce: 'pre', transform(_source, id) {
      const path = id.replaceAll('\\', '/');
      return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
    } }],
  });
  try {
    const registry = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    const kinds = ['faction-strength', 'factions', 'territory', 'territory-breakdown', 'wars', 'naval-losses'];
    registry.registerWidgets(await Promise.all(kinds.map(async kind => (await vite.ssrLoadModule(`/ui/src/widgets/${kind}/definition.ts`)).definition)));
    const Settings = (await vite.ssrLoadModule('/ui/src/dashboard/settings/WidgetSettings.tsx')).WidgetSettings as typeof WidgetSettings;
    const tracks = ['first', 'second'].map(roleId => ({ id: `${roleId}-track`, roleId, label: roleId, branchIndex: 0, anchorOffset: null }));
    const template: DashboardTemplate = { version: 1, name: 'Synthetic', alignment: 'calendar', roles: [], tracks, widgets: [], layouts: {} };
    const runtime = { defaultRoleId: 'first', client: {}, tracks: tracks.map(track => ({ ...track, source: { id: track.roleId, revision: '1' } })) } as unknown as WidgetRuntime;
    const widget: WidgetSpec = { id: 'chart', kind: 'faction-strength', configVersion: 1, title: 'Synthetic chart',
      campaign: 'default', trackIds: ['second-track'], countries: { kind: 'selected' }, time: { mode: 'campaign' } };
    const render = (spec = widget) => renderToStaticMarkup(createElement(Settings, { widget: spec, template, runtime, onChange() {} }));
    for (const kind of kinds.filter(kind => kind !== 'naval-losses')) {
      assert.match(render({ ...widget, kind }), /data-edited-campaign="first"/, `${kind} exposes the editor for the default campaign`);
    }
    assert.doesNotMatch(render({ ...widget, kind: 'naval-losses' }), /data-edited-campaign/, 'a chart without faction colours has no editor');
    const fixed = { ...widget, campaign: 'tracks' as const };
    assert.match(render(fixed), /data-edited-campaign="second"/, 'a fixed chart edits its own campaign');
    runtime.defaultRoleId = 'second';
    assert.match(render(), /data-edited-campaign="second"/, 'a default chart follows the new dashboard campaign');
    runtime.tracks = runtime.tracks.map(track => track.roleId === 'second'
      ? { ...track, source: { ...track.source, id: 'replacement' } } as BoundTrack : track);
    assert.match(render(fixed), /data-edited-campaign="replacement"/, 'reopening the source rebinds the editor');
    runtime.tracks = runtime.tracks.filter(track => track.roleId !== 'second');
    assert.doesNotMatch(render(fixed), /data-edited-campaign/);
    assert.match(render(fixed), /Load a campaign to edit its faction colours/, 'an unloaded source cannot edit another campaign');
  } finally { await vite.close(); }
});
