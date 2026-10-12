import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';

test('frame subtitles identify only campaigns different from the sidebar and warnings use the resolved campaign', async () => {
  const replacements: Record<string, string> = {
    '/dashboard/widgets.tsx': 'export const WidgetBody = () => null;',
    '/dashboard/WidgetSettingsMenu.tsx': 'export default props => { globalThis.__frameReset = props.onReset; return null; };',
    '/dashboard/definitions.ts': `export const widgetDefinition = kind => ({ icon: 'grid', data: kind !== 'decoration', version: 1, title: 'Synthetic',
      history: true, countrySelection: false, timeModes: ['follow', 'campaign'], defaultTime: 'follow', defaultCountries: { kind: 'tags', tags: [] } });`,
    '/dashboard/campaignSettings.ts': 'export const useCampaignSettings = () => ({ campaigns: {} });',
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: { factions: [{ id: '42', name: null,
      nameReason: 'Synthetic native naming refusal', names: [{ firstHour: 0, lastHour: 100, name: null, reason: 'Synthetic native naming refusal' }] }] } });`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'frame-render', enforce: 'pre',
      resolveId: id => id === 'react-grid-layout' ? '\0frame-grid' : undefined,
      load: id => id === '\0frame-grid' ? `export const useContainerWidth = () => ({ width: 1200, mounted: true });
        export const Responsive = ({ children }) => children;` : undefined,
      transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; },
    }] });
  try {
    const { default: Grid } = await vite.ssrLoadModule('/ui/src/dashboard/DashboardGrid.tsx') as typeof import('./DashboardGrid');
    const { saveRecordingNames, setRecordingName } = await vite.ssrLoadModule('/ui/src/dashboard/recordingNames.ts') as typeof import('./recordingNames.ts');
    const tracks = ['one', 'two'].map(roleId => ({ id: `track-${roleId}`, roleId, label: roleId,
      branch: { id: 'branch', firstHour: 0, lastHour: 100 },
      source: { path: `${roleId}.battleplan`, metadata: { countries: [], branches: [] } } })) as unknown as BoundTrack[];
    const runtime = { tracks, campaignTracks: tracks, defaultRoleId: 'one', maps: {}, client: {}, clock: { getHour: () => 0 } } as unknown as WidgetRuntime;
    const widget: WidgetSpec = { id: 'test', kind: 'synthetic', configVersion: 1, title: 'Synthetic', campaign: 'default',
      trackIds: ['track-one', 'stale-track'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: [] } };
    const template = { tracks, widgets: [widget], layouts: {} } as unknown as DashboardTemplate;
    const render = () => renderToStaticMarkup(createElement(Grid, { template, runtime, editing: false,
      onApply() {}, onRemove() {}, onLayouts() {} }));
    const frame = globalThis as typeof globalThis & { __frameReset?: () => void };
    render(); assert.equal(frame.__frameReset, undefined, 'a widget already at its defaults offers no reset');
    widget.title = 'Renamed'; render(); assert.equal(typeof frame.__frameReset, 'function');
    widget.title = 'Synthetic'; widget.contentScale = 125; render(); assert.equal(typeof frame.__frameReset, 'function');
    delete widget.contentScale;
    assert.doesNotMatch(render(), /dash-widget-subtitle|dash-inline-warning/, 'default mode ignores stale saved tracks');
    assert.match(render(), /data-tooltip="Show whole campaign"/);
    widget.trackIds = ['track-one'];
    widget.campaign = 'tracks';
    assert.doesNotMatch(render(), /dash-widget-subtitle/, 'an explicit assignment to the current campaign needs no subtitle');
    runtime.tracks = [tracks[0]];
    assert.doesNotMatch(render(), /dash-widget-subtitle/, 'one current campaign needs no subtitle');
    runtime.tracks = tracks;
    widget.trackIds = ['track-two'];
    assert.match(render(), /dash-widget-subtitle">two</);
    saveRecordingNames(setRecordingName({}, 'TWO.battleplan', 'Weekend campaign'), { getItem: () => null, setItem() {} });
    assert.match(render(), /dash-widget-subtitle">Weekend campaign</);
    saveRecordingNames({}, { getItem: () => null, setItem() {} });
    assert.match(render(), /dash-widget-subtitle">two</);
    runtime.tracks = [tracks[1]];
    assert.match(render(), /dash-widget-subtitle">two</, 'a different campaign is named even when it is the only loaded track');
    runtime.tracks = tracks;
    runtime.defaultRoleId = 'two';
    assert.doesNotMatch(render(), /dash-widget-subtitle/, 'switching the sidebar to the assigned campaign hides its subtitle');
    widget.trackIds = ['track-one'];
    assert.match(render(), /dash-widget-subtitle">one</);
    widget.campaign = 'default';
    assert.doesNotMatch(render(), /dash-widget-subtitle/, 'default widgets follow the new sidebar campaign');
    widget.campaign = 'tracks';
    widget.trackIds = ['opaque-missing-id'];
    assert.doesNotMatch(render(), /dash-widget-subtitle|opaque-missing-id/, 'a missing track supplies neither a filename nor a raw ID');
    widget.trackIds = ['track-one', 'opaque-missing-id'];
    assert.match(render(), /Campaign unavailable/);
    assert.doesNotMatch(render(), /opaque-missing-id/);
    widget.trackIds = ['opaque-missing-id', 'track-one'];
    assert.doesNotMatch(render(), /dash-widget-subtitle/, 'another loaded track cannot name a missing primary campaign');
    widget.campaign = 'default';
    runtime.defaultRoleId = 'unloaded-role';
    assert.doesNotMatch(render(), /dash-widget-subtitle|dash-inline-warning/);
    widget.campaign = 'tracks';
    widget.trackIds = ['track-one'];
    widget.kind = 'decoration';
    assert.doesNotMatch(render(), /dash-widget-subtitle/);
    widget.kind = 'synthetic';
    widget.time = { mode: 'campaign' };
    assert.match(render(), /data-tooltip="Follow playback"/);
    assert.match(render(), /aria-checked="false"/);

    const { default: Chip } = await vite.ssrLoadModule('/ui/src/dashboard/DatedCountryChip.tsx') as typeof import('./DatedCountryChip');
    const chip = (label: Parameters<typeof Chip>[0]['label'], reason: string | null = null, hour: number | null = 43800024) =>
      renderToStaticMarkup(createElement(Chip, { label, reason, hour }));
    assert.match(chip({ name: 'TAG' }), />Loading…</);
    assert.doesNotMatch(chip({ name: 'TAG' }), /Name unavailable/);
    assert.match(chip({ name: 'TAG', known: false }), />Unavailable</);
    assert.match(chip({ name: 'TAG', nameFailed: true }), /Name unavailable/);
    assert.doesNotMatch(chip(null, 'Synthetic native refusal'), /Synthetic native refusal/);
    assert.match(chip({ name: 'Name', known: true }), /As of /);
    assert.doesNotMatch(chip({ name: '', known: true }), /Loading…|Unavailable/);
    assert.match(chip({ name: 'Name', known: true }, null, null), />Unavailable</);

    const { default: CountryChip } = await vite.ssrLoadModule('/ui/src/dashboard/CountryChip.tsx') as typeof import('./CountryChip');
    runtime.tracks = [tracks[0]];
    const countryNames = ['Alpha', 'Beta', 'Gamma', 'Delta', 'Epsilon'];
    tracks[0].source.metadata.countries = countryNames.map((name, id) => ({ id: String(id), name, tag: String(id), observedBranchId: 'branch', observedHour: 0 }));
    for (const count of [1, 3, 5]) {
      const header = renderToStaticMarkup(createElement(CountryChip, { widget: { ...widget, trackIds: ['track-one'],
        countries: { kind: 'tags', tags: countryNames.slice(0, count).map((_, i) => String(i)) } }, runtime, active: false }));
      if (count === 1) assert.doesNotMatch(header, /data-tooltip=/);
      else assert.match(header, count === 3 ? /data-tooltip="Alpha, Beta, Gamma"/ : /data-tooltip="Alpha, Beta, Gamma and 2 more"/);
      assert.doesNotMatch(header, /Delta|Epsilon/);
    }
    const tags = Array.from({ length: 5000 }, (_, index) => `country-${index}`);
    tracks[0].source.metadata.countries = tags.map((tag, id) => ({ id: String(id), tag, name: `Name ${id}`, observedBranchId: 'branch', observedHour: 0 }));
    const large = renderToStaticMarkup(createElement(CountryChip, { widget: { ...widget, trackIds: ['track-one'],
      countries: { kind: 'tags', tags } }, runtime, active: false }));
    assert.match(large, /data-tooltip="Name 0, Name 1, Name 2 and 4997 more"/);
    assert.match(large, />\+4999</);
    const faction = renderToStaticMarkup(createElement(CountryChip, { widget: { ...widget, trackIds: ['track-one'],
      countries: { kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour', aggregate: false } }, runtime, active: false }));
    assert.match(faction, /Unnamed faction/);
    assert.doesNotMatch(faction, /Synthetic native naming refusal|data-tooltip=/);
  } finally { await vite.close(); delete (globalThis as { __frameReset?: unknown }).__frameReset; }
});
