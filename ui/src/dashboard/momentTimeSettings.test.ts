import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model';

interface TimeChoice { options: { value: string }[]; onChange: (mode: string) => void }

test('actual Air and Scoreboard moment settings expose Pin and save the resolved playback moment', async () => {
  const host = globalThis as typeof globalThis & { __momentTimeChoices?: TimeChoice[] };
  const fixtures: Record<string, string> = {
    '/dashboard/CampaignPicker.tsx': 'export default () => null;',
    '/dashboard/settings/CountriesSection.tsx': 'export const CountriesSection = () => null;',
    '/components/Segmented.tsx': `export const Segmented = props => {
      if (props.label === 'Time') globalThis.__momentTimeChoices.push(props); return null;
    };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'moment-time-settings', enforce: 'pre',
      transform: (_source, id) => Object.entries(fixtures).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const { registerWidgets, widgetUnavailable } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts') as typeof import('./definitions.ts');
    const { createWidget, resolveWindow } = await vite.ssrLoadModule('/ui/src/dashboard/model.ts') as typeof import('./model.ts');
    const { WidgetSettings } = await vite.ssrLoadModule('/ui/src/dashboard/settings/WidgetSettings.tsx') as typeof import('./settings/WidgetSettings.tsx');
    const { definition: air } = await vite.ssrLoadModule('/ui/src/widgets/air-superiority-region/definition.ts') as typeof import('../widgets/air-superiority-region/definition.ts');
    const { definition: scoreboard } = await vite.ssrLoadModule('/ui/src/widgets/scoreboard/definition.ts') as typeof import('../widgets/scoreboard/definition.ts');
    const { definition: placeholder } = await vite.ssrLoadModule('/ui/src/widgets/placeholder/definition.ts') as typeof import('../widgets/placeholder/definition.ts');
    registerWidgets([air, scoreboard, placeholder]);
    const template: DashboardTemplate = { version: 1, name: 'Synthetic', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: null }], widgets: [], layouts: {} };
    const track = { ...template.tracks[0], axisOffset: 1000, branch: { firstHour: 100, lastHour: 200 } } as BoundTrack;
    let position = -850;
    const runtime = { campaignTracks: template.tracks, defaultRoleId: 'primary', tracks: [track],
      clock: { getSnapshot: () => ({ position }) } } as unknown as WidgetRuntime;
    for (const definition of [air, scoreboard]) {
      assert.equal(definition.history, false);
      const widget = createWidget(definition.kind, 'main', 'moment');
      assert.deepEqual(widget.time, { mode: 'follow' });
      const saves: { widget: WidgetSpec; view?: boolean }[] = [];
      host.__momentTimeChoices = [];
      const markup = renderToStaticMarkup(createElement(WidgetSettings, { widget, template, runtime,
        onChange: (widget, view) => saves.push({ widget, view }) }));
      assert.match(markup, /aria-label="Time"/);
      assert.deepEqual(host.__momentTimeChoices.map(choice => choice.options.map(mode => mode.value)), [['follow', 'pin']]);
      host.__momentTimeChoices[0].onChange('pin');
      assert.equal(saves.length, 1);
      assert.equal(saves[0].view, true);
      assert.deepEqual(saves[0].widget.time, { mode: 'pin', windows: { main: { firstOffset: 50, lastOffset: 50 } } });
      assert.equal(widgetUnavailable(saves[0].widget), null);
      position = -825;
      assert.deepEqual(resolveWindow(saves[0].widget, track, [track], position), { firstHour: 150, lastHour: 150 });
      position = -850;
    }
    host.__momentTimeChoices = [];
    const markup = renderToStaticMarkup(createElement(WidgetSettings, { widget: createWidget(placeholder.kind, 'main', 'empty'),
      template, runtime, onChange() {} }));
    assert.doesNotMatch(markup, /aria-label="Time"/);
    assert.deepEqual(host.__momentTimeChoices, []);
  } finally {
    delete host.__momentTimeChoices;
    await vite.close();
  }
});
