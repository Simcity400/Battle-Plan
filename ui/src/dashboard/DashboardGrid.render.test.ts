import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';

const replacements: Record<string, string> = {
  '/dashboard/widgets.tsx': `import { useContext } from 'react'; import { HeldFailuresContext } from './heldFailures.ts';
    export const WidgetBody = () => { globalThis.__frameFailures = useContext(HeldFailuresContext); return null; };`,
  '/dashboard/WidgetSettingsMenu.tsx': 'export default props => { globalThis.__frameReset = props.onReset; return null; };',
  '/dashboard/definitions.ts': `export const registerWidgets = () => {}; export const widgetDefinition = kind => kind === 'follow-only'
    ? ({ icon: 'grid', data: true, version: 1, title: 'Follow only', history: false, countrySelection: false, timeModes: ['follow'], defaultTime: 'follow', defaultCountries: { kind: 'tags', tags: [] } })
    : kind === 'switched'
    ? ({ icon: 'grid', data: true, version: 1, title: 'Switched', history: true, countrySelection: true, timeModes: ['follow', 'campaign', 'pin', 'range'], defaultTime: 'follow', defaultCountries: { kind: 'tags', tags: [] },
        options: [{ key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'a', choices: [{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }] }],
        viewSwitch: { option: 'view' } })
    : kind === 'menu-switched'
    ? ({ icon: 'grid', data: true, version: 1, title: 'Many views', history: true, countrySelection: false, timeModes: ['follow', 'campaign'], defaultTime: 'follow', defaultCountries: { kind: 'tags', tags: [] },
        options: [{ key: 'pool', section: 'options', label: 'Pool', kind: 'choice', default: 'a', choices: [{ value: 'a', label: 'One' }, { value: 'b', label: 'Two' }, { value: 'c', label: 'Three' }, { value: 'd', label: 'Four' }] }],
        viewSwitch: { option: 'pool' } })
    : ({ icon: 'grid', data: kind !== 'decoration', version: 1, title: 'Synthetic',
        history: true, countrySelection: false, timeModes: ['follow', 'campaign', 'pin', 'range'], defaultTime: 'follow', defaultCountries: { kind: 'tags', tags: [] } });`,
  '/dashboard/campaignSettings.ts': "export const useCampaignSettings = () => ({ campaigns: {} }); export const useFactionStylesKey = () => '';",
  '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: { factions: [{ id: '42', name: null,
    nameReason: 'Synthetic native naming refusal', names: [{ firstHour: 0, lastHour: 100, name: null, reason: 'Synthetic native naming refusal' }] }] } });`,
  '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, track) => ({ labelAt: () => tag => {
    const entry = track?.source?.metadata?.countries?.find(item => item.tag === tag);
    return entry ? { name: entry.name, known: true, flag: null, flagPending: false } : { name: tag, known: false, flag: null, flagPending: false };
  }, revision: 1 });`,
};
const frameServer = () => createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'frame-render', enforce: 'pre',
    resolveId: id => id === 'react-grid-layout' ? '\0frame-grid' : undefined,
    load: id => id === '\0frame-grid' ? `export const useContainerWidth = () => ({ width: 1200, mounted: true });
      export const Responsive = ({ children }) => children;` : undefined,
    transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; },
  }] });
const syntheticTracks = () => ['one', 'two'].map(roleId => ({ id: `track-${roleId}`, roleId, label: roleId, axisOffset: 0,
  branch: { id: 'branch', firstHour: 43800000, lastHour: 43801000, gaps: [] },
  source: { id: `src-${roleId}`, revision: '1', path: `${roleId}.battleplan`, metadata: { countries: [], branches: [] } } })) as unknown as BoundTrack[];

test('frame subtitles identify only campaigns different from the sidebar, warnings use the resolved campaign, and the time chip reflects all four modes', async () => {
  const vite = await frameServer();
  try {
    const { default: Grid } = await vite.ssrLoadModule('/ui/src/dashboard/DashboardGrid.tsx') as typeof import('./DashboardGrid');
    const tracks = syntheticTracks();
    const runtime = { tracks, campaignTracks: tracks, defaultRoleId: 'one', maps: {}, client: {}, clock: { getHour: () => 43800024 } } as unknown as WidgetRuntime;
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
    const liveHtml = render();
    assert.match(liveHtml, /<button[^>]*dash-time-chip[^>]*>.*Live.*<\/button>/);
    assert.doesNotMatch(liveHtml, /live-toggle/);
    widget.trackIds = ['track-one'];
    widget.campaign = 'tracks';
    assert.doesNotMatch(render(), /dash-widget-subtitle/, 'an explicit assignment to the current campaign needs no subtitle');
    runtime.tracks = [tracks[0]];
    assert.doesNotMatch(render(), /dash-widget-subtitle/, 'one current campaign needs no subtitle');
    runtime.tracks = tracks;
    widget.trackIds = ['track-two'];
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
    assert.doesNotMatch(render(), /dash-widget-subtitle|dash-time-chip/);
    widget.kind = 'synthetic';
    widget.time = { mode: 'campaign' };
    assert.match(render(), /dash-time-chip[^>]*>.*Whole campaign.*<\/button>/);
    widget.time = { mode: 'pin', windows: { 'track-one': { firstOffset: 24, lastOffset: 24 } } };
    assert.match(render(), /Pinned · /);
    widget.time = { mode: 'range', firstOffset: 0, lastOffset: 720 };
    assert.match(render(), /–/);
    widget.kind = 'follow-only';
    widget.time = { mode: 'follow' };
    const staticHtml = render();
    assert.match(staticHtml, /<span[^>]*dash-time-chip is-static[^>]*>.*Live.*<\/span>/);
    assert.doesNotMatch(staticHtml, /<button[^>]*dash-time-chip/);

    const { default: CountryChip } = await vite.ssrLoadModule('/ui/src/dashboard/CountryChip.tsx') as typeof import('./CountryChip');
    runtime.tracks = [tracks[0]];
    const chipWidget = { ...widget, kind: 'synthetic', time: { mode: 'campaign' as const }, trackIds: ['track-one'] };
    const countryNames = ['Alpha', 'Beta', 'Gamma', 'Delta', 'Epsilon'];
    tracks[0].source.metadata.countries = countryNames.map((name, id) => ({ id: String(id), name, tag: String(id), observedBranchId: 'branch', observedHour: 43801000 }));
    for (const count of [1, 3, 5]) {
      const header = renderToStaticMarkup(createElement(CountryChip, { widget: { ...chipWidget,
        countries: { kind: 'tags', tags: countryNames.slice(0, count).map((_, i) => String(i)) } }, runtime, active: false }));
      if (count === 1) assert.doesNotMatch(header, /<span class="dash-widget-country" data-tooltip=/);
      else assert.match(header, count === 3 ? /data-tooltip="Loading…, Loading…, Loading…"/ : /data-tooltip="Loading…, Loading…, Loading… and 2 more"/);
      assert.doesNotMatch(header, /Alpha|Beta|Gamma|Delta|Epsilon/, 'names come from the dated identity, never from the recording metadata');
    }
    const tags = Array.from({ length: 5000 }, (_, index) => `country-${index}`);
    tracks[0].source.metadata.countries = tags.map((tag, id) => ({ id: String(id), tag, name: `Name ${id}`, observedBranchId: 'branch', observedHour: 43801000 }));
    const large = renderToStaticMarkup(createElement(CountryChip, { widget: { ...chipWidget,
      countries: { kind: 'tags', tags } }, runtime, active: false }));
    assert.match(large, /data-tooltip="Loading…, Loading…, Loading… and 4997 more"/);
    assert.match(large, />\+4999</);
    const faction = renderToStaticMarkup(createElement(CountryChip, { widget: { ...chipWidget,
      countries: { kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour', aggregate: false } }, runtime, active: false }));
    assert.match(faction, /Loading faction…/);
    assert.doesNotMatch(faction, /Synthetic native naming refusal|data-tooltip=/);
  } finally { await vite.close(); delete (globalThis as { __frameReset?: unknown }).__frameReset; }
});

test('header order, view switch slot, narrow drop order, and settings section order match the frame standard', async () => {
  const vite = await frameServer();
  try {
    const { default: Grid } = await vite.ssrLoadModule('/ui/src/dashboard/DashboardGrid.tsx') as typeof import('./DashboardGrid');
    const { headerDropStage } = await vite.ssrLoadModule('/ui/src/dashboard/widgetFrame.tsx') as typeof import('./widgetFrame');
    const { SECTION_TITLES, settingsSections } = await vite.ssrLoadModule('/ui/src/dashboard/settingsSections.ts') as typeof import('./settingsSections');
    const tracks = syntheticTracks();
    const runtime = { tracks, campaignTracks: tracks, defaultRoleId: 'one', maps: {}, client: {}, clock: { getHour: () => 43800024 } } as unknown as WidgetRuntime;

    const switched: WidgetSpec = { id: 'sw', kind: 'switched', configVersion: 1, title: 'Switched', campaign: 'default',
      trackIds: ['track-one'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: ['GER'] } };
    const html = renderToStaticMarkup(createElement(Grid, { template: { tracks, widgets: [switched], layouts: {} } as unknown as DashboardTemplate,
      runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    const positions = [
      html.indexOf('dash-widget-icon'),
      html.indexOf('dash-widget-title'),
      html.indexOf('dash-widget-summary'),
      html.indexOf('dash-widget-country-button'),
      html.indexOf('dash-view-switch'),
      html.indexOf('dash-time-chip'),
    ];
    assert.ok(positions.every(pos => pos >= 0), 'every header slot renders');
    for (let i = 1; i < positions.length; i++) assert.ok(positions[i] > positions[i - 1], `slot ${i} follows slot ${i - 1}`);
    assert.match(html, /role="group"[^>]*aria-label="Switched view"/);

    const many: WidgetSpec = { id: 'many', kind: 'menu-switched', configVersion: 1, title: 'Many views', campaign: 'default',
      trackIds: ['track-one'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: [] } };
    const manyHtml = renderToStaticMarkup(createElement(Grid, { template: { tracks, widgets: [many], layouts: {} } as unknown as DashboardTemplate,
      runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    assert.match(manyHtml, /dash-view-menu-trigger[^>]*aria-haspopup="menu"/, 'four or more choices render as a menu');

    const measured = {
      fixed: 44, titleFull: 120, hasSummary: true, pill: 110,
      hasViewSwitch: true, viewMenuOnly: false, viewFull: 160, viewMenu: 84,
      hasSubject: true, subjectFull: 120, subjectCompact: 42,
      hasTime: true, timeFull: 140, timeCompact: 24,
    };
    const wide = headerDropStage(760, measured);
    assert.deepEqual(wide, { stage: 0, dropPill: false, collapseView: false, compactSubject: false, compactTime: false },
      'a header with room keeps full subject name, segmented switch, summary pill, and full time label');
    assert.deepEqual(headerDropStage(504, {
      fixed: 44, titleFull: 84, hasSummary: false, pill: 0,
      hasViewSwitch: false, viewFull: 0, viewMenu: 0,
      hasSubject: true, subjectFull: 116, subjectCompact: 42,
      hasTime: true, timeFull: 172, timeCompact: 24,
    }), { stage: 0, dropPill: false, collapseView: false, compactSubject: false, compactTime: false },
      'Game score at 520 px keeps full subject name and full date range when it fits');
    assert.deepEqual(headerDropStage(774, {
      fixed: 44, titleFull: 150, hasSummary: false, pill: 0,
      hasViewSwitch: true, viewMenuOnly: false, viewFull: 166, viewMenu: 84,
      hasSubject: true, subjectFull: 126, subjectCompact: 42,
      hasTime: true, timeFull: 66, timeCompact: 24,
    }), { stage: 0, dropPill: false, collapseView: false, compactSubject: false, compactTime: false },
      'Army losses by month at 790 px keeps segmented switch and full subject name');
    const stages = [760, 620, 550, 470, 260].map(w => headerDropStage(w, measured));
    assert.deepEqual(stages.map(s => s.stage), [0, 1, 2, 3, 4], 'each drop stage triggers only when the previous stage overflows');
    assert.deepEqual(stages[4], { stage: 4, dropPill: true, collapseView: true, compactSubject: true, compactTime: true });
    assert.equal(headerDropStage(440, measured).stage, 4, 'the time chip collapses before the title is cut');
    assert.equal(headerDropStage(520, { ...measured, filterFull: 110, filterCompact: 46 }).stage, 3, 'a header filter collapses with the subject chip');
    assert.deepEqual(headerDropStage(470, { ...measured, timeLive: true }), { stage: 3, dropPill: true, collapseView: true, compactSubject: true, compactTime: true },
      'a live time chip collapses to its dot with the subject chip');
    assert.deepEqual(headerDropStage(300, {
      fixed: 44, titleFull: 84, hasSummary: true, pill: 95,
      hasViewSwitch: false, viewFull: 0, viewMenu: 0,
      hasSubject: false, subjectFull: 0, subjectCompact: 0,
      hasTime: true, timeLive: true, timeFull: 70, timeCompact: 44,
    }), { stage: 1, dropPill: true, collapseView: false, compactSubject: false, compactTime: true },
      'a live time chip collapses to its dot when the count is dropped, as at the minimum size');

    const { timeChipSummary } = await vite.ssrLoadModule('/ui/src/dashboard/WidgetTimeChip.tsx') as typeof import('./WidgetTimeChip');
    const { engineHour } = await vite.ssrLoadModule('/ui/src/dashboard/calendar.ts') as typeof import('./calendar');
    const start1939 = engineHour({ year: 1939, month: 1, day: 1, hour: 12 })!;
    const nov1939 = engineHour({ year: 1939, month: 11, day: 4, hour: 12 })!;
    const jan4 = engineHour({ year: 1939, month: 1, day: 4, hour: 12 })!;
    const rangeTrack = { ...tracks[0], branch: { ...tracks[0].branch, firstHour: start1939, lastHour: nov1939 } } as BoundTrack;
    const rangeWidget: WidgetSpec = { ...switched, time: { mode: 'range', firstOffset: 0, lastOffset: nov1939 - start1939 } };
    assert.equal(timeChipSummary(rangeWidget, rangeWidget, rangeTrack, [rangeTrack], start1939).label, '1 Jan – 4 Nov 1939');
    const sameMonthWidget: WidgetSpec = { ...switched, time: { mode: 'range', firstOffset: 0, lastOffset: jan4 - start1939 } };
    assert.equal(timeChipSummary(sameMonthWidget, sameMonthWidget, rangeTrack, [rangeTrack], start1939).label, '1–4 Jan 1939');
    const rangeHtml = renderToStaticMarkup(createElement(Grid, {
      template: { tracks: [rangeTrack], widgets: [rangeWidget], layouts: {} } as unknown as DashboardTemplate,
      runtime: { ...runtime, tracks: [rangeTrack], campaignTracks: [rangeTrack] }, editing: false, onApply() {}, onRemove() {}, onLayouts() {},
    }));
    assert.match(rangeHtml, /1 Jan – 4 Nov 1939/);
    assert.match(rangeHtml, /role="group"[^>]*aria-label="Switched view"/);

    const mapSections = settingsSections({ kind: 'map', data: true, history: false, countrySelection: true, timeModes: ['follow', 'pin'],
      defaultTime: 'follow', factionColours: true, options: [
        { key: 'a', section: 'options', label: 'A', kind: 'toggle', default: false },
        { key: 'b', section: 'axes', label: 'B', kind: 'toggle', default: false },
        { key: 'c', section: 'interaction', label: 'C', kind: 'toggle', default: false },
      ] } as never, { a: false, b: false, c: false }, { kind: 'all' }, 'follow');
    assert.deepEqual(mapSections, ['campaign', 'countries', 'time', 'size', 'options', 'map', 'factionColours', 'axes', 'interaction']);
    assert.equal(SECTION_TITLES.factionColours, 'Colours');
    assert.equal(SECTION_TITLES.map, 'Map');
    assert.equal(rangeHtml.includes('dash-widget-caption-slot'), false, 'notes never take a body caption slot');
    const { FamilyHeaderSummary } = await vite.ssrLoadModule('/ui/src/families/host/FamilyHeaderSummary.tsx') as typeof import('../families/host/FamilyHeaderSummary');
    const { resolveHeaderSummary } = await vite.ssrLoadModule('/ui/src/families/host/headerSummary.ts') as typeof import('../families/host/headerSummary');
    const noteHtml = renderToStaticMarkup(createElement(FamilyHeaderSummary, {
      summary: resolveHeaderSummary({ notes: ['Loss totals include corrections.'] }),
    }));
    assert.match(noteHtml, /class="fh-summary fh-summary-partial" tabindex="0" data-tooltip="Loss totals include corrections\." aria-label="Note\. Loss totals include corrections\.">Note<span class="fh-summary-hatch" aria-hidden="true"><\/span><\/span>/);
  } finally { await vite.close(); }
});

test('every registered widget kind preserves its default and saved time modes', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    await vite.ssrLoadModule('/ui/src/widgets/index.ts');
    const { widgetDefinitions } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts') as typeof import('./definitions');
    const { createWidget } = await vite.ssrLoadModule('/ui/src/dashboard/model.ts') as typeof import('./model');
    const { parseTemplate } = await vite.ssrLoadModule('/ui/src/dashboard/storage.ts') as typeof import('./storage');
    const definitions = widgetDefinitions();
    assert.ok(definitions.length >= 100);
    for (const def of definitions) {
      const created = createWidget(def.kind, 'track-1', `w-${def.kind}`);
      const expectedDefault = def.defaultTime ?? def.timeModes[0] ?? 'follow';
      assert.equal(created.time.mode, expectedDefault, `${def.kind} default time matches definition`);
      for (const mode of def.timeModes) {
        const saved = mode === 'pin' ? { mode: 'pin' as const, windows: { 'track-1': { firstOffset: 24, lastOffset: 24 } } }
          : mode === 'range' ? { mode: 'range' as const, firstOffset: 0, lastOffset: 48 }
            : { mode };
        const parsed = parseTemplate({
          version: 1, name: 'Saved', alignment: 'calendar',
          roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
          tracks: [{ id: 'track-1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null }],
          widgets: [{ ...created, time: saved }],
          layouts: { lg: [{ i: created.id, x: 0, y: 0, w: 6, h: 6 }] },
        });
        assert.ok(parsed, `${def.kind} parses with saved time mode ${mode}`);
        assert.equal(parsed.widgets[0]?.time.mode, mode, `${def.kind} saved time mode ${mode} is preserved`);
      }
    }
  } finally { await vite.close(); }
});

test('a widget header draws its kind\'s own icon and colour, and an image in its own colours', async () => {
  const vite = await frameServer();
  try {
    const { default: Grid } = await vite.ssrLoadModule('/ui/src/dashboard/DashboardGrid.tsx') as typeof import('./DashboardGrid');
    const { default: Preview } = await vite.ssrLoadModule('/ui/src/dashboard/WidgetPreview.tsx') as typeof import('./WidgetPreview');
    const { WidgetIconsContext } = await vite.ssrLoadModule('/ui/src/shell/appearance/WidgetIconMenu.tsx') as typeof import('../shell/appearance/WidgetIconMenu');
    const { Icon } = await vite.ssrLoadModule('/ui/src/components/Icons.tsx') as typeof import('../components/Icons');
    const tracks = syntheticTracks();
    const runtime = { tracks, campaignTracks: tracks, defaultRoleId: 'one', maps: {}, client: {}, clock: { getHour: () => 0 } } as unknown as WidgetRuntime;
    const widget = (id: string, kind: string): WidgetSpec => ({ id, kind, configVersion: 1, title: 'Synthetic', campaign: 'default',
      trackIds: ['track-one'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: [] } });
    const template = { tracks, widgets: [widget('first', 'navy-size'), widget('second', 'navy-size'), widget('third', 'aces')], layouts: {} } as unknown as DashboardTemplate;
    const styles: Record<string, { icon?: string; colour?: string }> = { 'navy-size': { icon: 'anchor', colour: '#ff0000' } };
    const icons = { style: (kind: string) => styles[kind] ?? {}, openMenu() {}, edit() {}, reset() {} } as never;
    const grid = (provided = true) => {
      const element = createElement(Grid, { template, runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} });
      return renderToStaticMarkup(provided ? createElement(WidgetIconsContext.Provider, { value: icons }, element) : element);
    };
    const glyph = (name: 'anchor' | 'grid') => renderToStaticMarkup(createElement(Icon, { name }));
    const header = (markup: string, id: string) => markup.match(new RegExp(`<span class="dash-widget-icon"[^>]*data-widget-icon="${id}"[^>]*>(.*?)</span>`))?.[0] ?? '';

    for (const id of ['first', 'second']) {
      assert.match(header(grid(), id), /style="color:#ff0000"/, 'every copy of the kind takes its colour');
      assert.ok(header(grid(), id).includes(glyph('anchor')), 'every copy of the kind draws its icon');
    }
    assert.doesNotMatch(header(grid(), 'third'), /style=/, 'another kind keeps the muted default');
    assert.ok(header(grid(), 'third').includes(glyph('grid')));
    assert.ok(header(grid(false), 'first').includes(glyph('grid')), 'without the app, headers draw their defaults');

    styles['navy-size'] = { icon: 'data:image/png;base64,iVBORw0KGgo=' };
    assert.match(header(grid(), 'first'), /<img class="icon-image" src="data:image\/png;base64,iVBORw0KGgo=" alt="" aria-hidden="true" draggable="false"\/>/);
    assert.doesNotMatch(header(grid(), 'first'), /<svg/);

    styles['navy-size'] = { icon: 'anchor', colour: '#00ff00' };
    const definition = { kind: 'navy-size', icon: 'grid', title: 'Navy size', defaultSize: { w: 4, h: 8 }, timeModes: ['follow'], defaultTime: 'follow', data: true } as never;
    const preview = renderToStaticMarkup(createElement(WidgetIconsContext.Provider, { value: icons },
      createElement(Preview, { definition, runtime: { ...runtime, tracks: [], campaignTracks: [] } as WidgetRuntime })));
    assert.ok(preview.includes(renderToStaticMarkup(createElement(Icon, { name: 'anchor', style: { color: '#00ff00' } }))), 'the catalogue preview draws the kind\'s icon');
    const livePreview = renderToStaticMarkup(createElement(WidgetIconsContext.Provider, { value: icons },
      createElement(Preview, { definition, runtime })));
    assert.equal(livePreview.includes('dash-widget-caption-slot'), false, 'preview renders no body caption slot');
  } finally { await vite.close(); }
});

test('a frame says each failure its body holds beside published data once, above the body, with Retry', async () => {
  const vite = await frameServer();
  const host = globalThis as typeof globalThis & { __frameFailures?: unknown };
  try {
    const { default: Grid } = await vite.ssrLoadModule('/ui/src/dashboard/DashboardGrid.tsx') as typeof import('./DashboardGrid');
    const { default: Notices } = await vite.ssrLoadModule('/ui/src/dashboard/HeldFailureNotices.tsx') as typeof import('./HeldFailureNotices');
    const { HeldFailures } = await vite.ssrLoadModule('/ui/src/dashboard/heldFailures.ts') as typeof import('./heldFailures.ts');
    const tracks = syntheticTracks();
    const runtime = { tracks, campaignTracks: tracks, defaultRoleId: 'one', maps: {}, client: {}, clock: { getHour: () => 0 } } as unknown as WidgetRuntime;
    const widget: WidgetSpec = { id: 'held', kind: 'synthetic', configVersion: 1, title: 'Synthetic', campaign: 'default',
      trackIds: ['track-one'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: [] } };
    const template = { tracks, widgets: [widget], layouts: {} } as unknown as DashboardTemplate;
    renderToStaticMarkup(createElement(Grid, { template, runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    assert.ok(host.__frameFailures instanceof HeldFailures, 'the body is drawn inside its frame\'s failures');

    const failures = new HeldFailures();
    assert.equal(renderToStaticMarkup(createElement(Notices, { failures })), '', 'nothing failed, nothing said');
    const asked: string[] = [];
    failures.put({}, [{ error: new Error('Preparation stopped'), retry: () => asked.push('roster') }]);
    failures.put({}, [{ error: new Error('Preparation stopped'), retry: () => asked.push('labels') }]);
    failures.put({}, [{ error: new Error('Source released'), retry: () => asked.push('history') }]);
    const html = renderToStaticMarkup(createElement(Notices, { failures }));
    assert.equal(html.match(/data-detail="Preparation stopped"/g)?.length, 1, 'one failure is said once');
    assert.match(html, /data-detail="Source released"/);
    assert.equal(html.match(/>Retry</g)?.length, 2);
    failures.retry(new Error('Preparation stopped'));
    assert.deepEqual(asked, ['roster', 'labels'], 'its Retry asks every reader holding it again');
  } finally { await vite.close(); delete host.__frameFailures; }
});

test('catalogue previews respect field.visibleWhen, TimeSection stays usable without a campaign, and research/lazy widgets use WidgetState', async () => {
  const vite = await frameServer();
  try {
    const { default: Preview } = await vite.ssrLoadModule('/ui/src/dashboard/WidgetPreview.tsx') as typeof import('./WidgetPreview');
    const { TimeSection } = await vite.ssrLoadModule('/ui/src/dashboard/settings/TimeSection.tsx') as typeof import('./settings/TimeSection');
    const tracks = syntheticTracks();
    const runtime = { tracks, campaignTracks: tracks, defaultRoleId: 'one', maps: {}, client: {}, clock: { getHour: () => 43800024 } } as unknown as WidgetRuntime;

    const conditionalDef = {
      kind: 'conditional-switch', icon: 'grid', title: 'Stockpile', defaultSize: { w: 4, h: 8 },
      timeModes: ['follow'], defaultTime: 'follow', data: true,
      options: [
        { key: 'mode', section: 'options', label: 'Mode', kind: 'choice', default: 'table', choices: [{ value: 'table', label: 'Table' }, { value: 'over-time', label: 'Over time' }] },
        { key: 'metric', section: 'options', label: 'Metric', kind: 'choice', default: 'count', choices: [{ value: 'count', label: 'Count' }, { value: 'ic', label: 'IC' }],
          visibleWhen: (opts: Record<string, unknown>) => opts.mode !== 'over-time' },
      ],
      viewSwitch: { option: 'metric' },
    } as never;
    const shownPreview = renderToStaticMarkup(createElement(Preview, { definition: conditionalDef, initial: { title: 'Stockpile', options: { mode: 'table' } }, runtime }));
    assert.match(shownPreview, /data-view-switch=""/, 'preview shows declared view switch when visibleWhen passes');
    const hiddenPreview = renderToStaticMarkup(createElement(Preview, { definition: conditionalDef, initial: { title: 'Stockpile', options: { mode: 'over-time' } }, runtime }));
    assert.doesNotMatch(hiddenPreview, /data-view-switch/, 'preview hides declared view switch when visibleWhen hides the option');

    const unloadedWidget: WidgetSpec = { id: 't', kind: 'synthetic', configVersion: 1, title: 'Synthetic', campaign: 'default',
      trackIds: [], time: { mode: 'follow' }, countries: { kind: 'all' } };
    const timeHtml = renderToStaticMarkup(createElement(TimeSection, {
      widget: unloadedWidget, effective: unloadedWidget, reference: undefined,
      runtime: { ...runtime, tracks: [], campaignTracks: [] } as WidgetRuntime,
      modes: ['follow', 'pin'], onChange() {},
    }));
    assert.match(timeHtml, />Live</);
    assert.match(timeHtml, /aria-disabled="true"[^>]*data-tooltip="Load a campaign to pin a moment\."[^>]*>Pinned</);
    assert.doesNotMatch(timeHtml, /disabled=""[^>]*>Pinned</, 'unavailable time mode stays focusable with its tooltip');
    assert.match(timeHtml, /Load a campaign to pin a moment\./, 'unloaded time popover explains why pinning is unavailable');
  } finally { await vite.close(); }

  const realVite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'widget-state-ssr', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/follow/useFollowed.ts')) {
          return source.replace(/export function usePipeline\(/, 'function real_usePipeline(') + '\nexport function usePipeline() { return null; }';
        }
        if (path.endsWith('/charts/useChartTokens.ts')) return 'export const useChartTokens = () => ({ categories: ["#777"] }); export const useViewportHeight = () => 720;';
      },
    }] });
  try {
    const { lazyWidget } = await realVite.ssrLoadModule('/ui/src/widgets/lazyWidget.tsx') as typeof import('../widgets/lazyWidget');
    const lazy = lazyWidget('never-loaded-synthetic-kind', ['render']);
    const fallbackHtml = renderToStaticMarkup(createElement(lazy.render! as never, {} as never));
    assert.match(fallbackHtml, /data-state="loading"/, 'lazy widget fallback renders WidgetState loading');
    assert.doesNotMatch(fallbackHtml, /dash-empty|role="status"/);

    const { default: ActiveResearch } = await realVite.ssrLoadModule('/ui/src/widgets/active-research/ActiveResearchView.tsx');
    const { activeResearchDeclaration } = await realVite.ssrLoadModule('/ui/src/widgets/active-research/activeResearchProjection.ts');
    const emptyRuntime = { tracks: [], campaignTracks: [], defaultRoleId: null, maps: {}, client: { available: true },
      clock: { getHour: () => 0, getSnapshot: () => ({ playing: false, hoursPerSecond: 1 }), subscribe: () => () => {}, subscribeFrame: () => () => {} } } as unknown as WidgetRuntime;
    const baseWidget = { id: 'r', kind: 'active-research', configVersion: 1, title: 'Active research', campaign: 'default', trackIds: [], time: { mode: 'follow' }, countries: { kind: 'all' }, options: {} } as WidgetSpec;
    const html = renderToStaticMarkup(createElement(ActiveResearch as never, { widget: baseWidget, options: {}, manifestDeclaration: activeResearchDeclaration, runtime: emptyRuntime, active: true, mapOptions: { fog: true, country: null }, headerSlot: null, onDiagnostics() {} } as never));
    assert.match(html, /data-state="empty"/, 'active-research routes empty state through WidgetState');
    assert.doesNotMatch(html, /dash-empty/, 'active-research does not render raw dash-empty');

    const { default: WidgetViewSwitch } = await realVite.ssrLoadModule('/ui/src/dashboard/WidgetViewSwitch.tsx');
    const roSwitch = renderToStaticMarkup(createElement(WidgetViewSwitch as never, {
      label: 'Mode', value: 'a', options: [{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }], readOnly: true, onChange() {},
    } as never));
    assert.match(roSwitch, /<button[^>]*disabled=""[^>]*aria-disabled="true"/, 'read-only segmented view switch disables its buttons');
  } finally { await realVite.close(); }

  const scoreboardVite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'scoreboard-follow-ssr', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/products.ts')) {
          return `export const useProduct = (_client, request) => request?.product === 'scoreboard-columns' ? globalThis.__sbAux : { data: undefined, error: undefined, retry() {} };
            export const useFollowedProduct = () => ({ data: undefined, error: undefined, retry() {} });`;
        }
        if (path.endsWith('/dashboard/useCountryGroups.ts')) {
          return `export const useCountryGroups = () => ({ data: globalThis.__datedTestGroups ?? { countryIdentities: [{ tag: 'GER' }], warnings: [] }, complete: true, error: undefined, retry() {} });`;
        }
        if (path.endsWith('/families/identity/react.ts')) {
          return _source.replace(/export function useSlotIndex\(/, 'function realUseSlotIndex(')
            + '\nexport function useSlotIndex() { return { slots: { runsForKey: () => [], tagRuns: () => [] }, revision: 1, failure: undefined, retry() {} }; }';
        }
        if (path.endsWith('/dashboard/countryFlagsAt.ts')) {
          return `export const useCountryLabelsAt = () => ({ labelAt: () => () => ({ known: true, name: 'Germany' }), exactAt: () => () => ({ known: true, name: 'Germany' }) });`;
        }
        if (path.endsWith('/dashboard/countryNamesAt.ts')) {
          return `export const useHeldTimeline = () => undefined;
            export const useCountryNamesAt = () => ({ names: new Map(), error: undefined, nameAt: () => ({ known: true, name: 'Germany' }) });`;
        }
        if (path.endsWith('/dashboard/contentRevision.ts')) {
          return `export const useContentRevision = () => 0;`;
        }
        if (path.endsWith('/dashboard/regionGate.ts')) {
          return `export const useGate = () => ({ open: true, follow: q => q, checkFollow: q => globalThis.__sbFollowCheck(q), hour: 100, hourAt: () => 100, recorded: h => h, span: (a, b) => ({ firstHour: a, lastHour: b }) });`;
        }
        if (path.endsWith('/dashboard/widgetTime.ts')) {
          return `export const useRegion = () => ({}); export const useRegionCoverage = () => ({ outside: null }); export const useOutsideRecording = () => null;`;
        }
        if (path.endsWith('/families/host/FamilyHost.tsx')) {
          return `export default ({ blockedState }) => blockedState ?? null;`;
        }
      },
    }] });
  try {
    const { ShownError } = await scoreboardVite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    const { default: DeclaredFamilyWidget } = await scoreboardVite.ssrLoadModule('/ui/src/families/host/DeclaredFamilyWidget.tsx');
    const { tableHostAdapter } = await scoreboardVite.ssrLoadModule('/ui/src/families/table/hostAdapter.ts');
    const { scoreboardDeclaration } = await scoreboardVite.ssrLoadModule('/ui/src/widgets/scoreboard/scoreboardProjection.ts');
    await scoreboardVite.ssrLoadModule('/ui/src/widgets/definitions.ts');
    const { widgetViewOptions } = await scoreboardVite.ssrLoadModule('/ui/src/widgets/manifest.ts');
    const sbProps = {
      declaration: scoreboardDeclaration,
      options: widgetViewOptions('scoreboard', {}),
      adapter: tableHostAdapter,
      widget: { id: 'sb', kind: 'scoreboard', configVersion: 1, title: 'Scoreboard', campaign: 'default', trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: ['GER'] }, options: {} },
      runtime: {
        tracks: [{ id: 't1', roleId: 'primary', label: 'Primary', axisOffset: 0, branch: { id: '1', firstHour: 100, lastHour: 200 }, source: { id: 's1', revision: '1', metadata: { campaignKey: 'c', countries: [] } } }],
        campaignTracks: [], defaultRoleId: 'primary', maps: {}, client: { available: true },
        clock: { getHour: () => 100, getSnapshot: () => ({ playing: false, hoursPerSecond: 1 }), subscribe: () => () => {}, subscribeFrame: () => () => {} },
      },
      active: true, mapOptions: { fog: true, country: null }, headerSlot: null, onDiagnostics() {},
    };
    const okRegistry = { data: { columns: [{ key: 'factories', label: 'Factories', available: true, decimals: 0, family: { kind: 'country', measure: 'factories' } }], defaultColumns: ['factories'], defaultSort: 'factories', warnings: [] }, error: undefined, retry() {} };
    (globalThis as Record<string, unknown>).__sbAux = okRegistry;
    (globalThis as Record<string, unknown>).__sbFollowCheck = () => ({ query: null, refused: 'Table absent in format 141' });
    const refusedHtml = renderToStaticMarkup(createElement(DeclaredFamilyWidget as never, sbProps as never));
    assert.match(refusedHtml, /data-state="unavailable"/, 'scoreboard recording refusal renders unavailable, never a load error');
    assert.doesNotMatch(refusedHtml, /role="alert"|is-error|Retry/);
    (globalThis as Record<string, unknown>).__sbFollowCheck = (q: unknown) => ({ query: q, refused: null });
    const emptyPlan = { ...scoreboardDeclaration, sources: () => ({ owner: 'empty-plan', entries: [], refused: 'No territory is available for this selection.' }) };
    for (const time of [
      { mode: 'follow' },
      { mode: 'pin', windows: { t1: { firstOffset: 5, lastOffset: 5 } } },
      { mode: 'range', firstOffset: 5, lastOffset: 15 },
    ]) {
      const html = renderToStaticMarkup(createElement(DeclaredFamilyWidget as never, {
        ...sbProps, declaration: emptyPlan, widget: { ...sbProps.widget, time },
      } as never));
      assert.match(html, /data-state="unavailable"/, time.mode);
      assert.match(html, />No territory is available for this selection\.</, time.mode);
      assert.doesNotMatch(html, /data-state="loading"/, time.mode);
    }
    const { territoryLinesDeclaration } = await scoreboardVite.ssrLoadModule('/ui/src/widgets/territory/territoryLinesProjection.ts');
    const { timeChartHostAdapter } = await scoreboardVite.ssrLoadModule('/ui/src/families/time-chart/hostAdapter.ts');
    (globalThis as Record<string, unknown>).__datedTestGroups = {
      humans: [], factions: [], wars: [], countryIdentities: [], warnings: [],
      unobserved: { humans: [], factions: [], wars: [{ firstHour: 120, lastHour: 130 }], unattributed: [] },
    };
    for (const [hour, reason] of [[119, 'This group has no members yet.'], [120, 'Group membership was not recorded at this time.']] as const) {
      const html = renderToStaticMarkup(createElement(DeclaredFamilyWidget as never, {
        ...sbProps, declaration: territoryLinesDeclaration, adapter: timeChartHostAdapter,
        options: widgetViewOptions('territory', { view: 'lines' }),
        widget: { ...sbProps.widget, kind: 'territory', options: { view: 'lines' },
          countries: { kind: 'group', group: { type: 'war', tag: 'GER' }, membership: 'ever', aggregate: false } },
        runtime: { ...sbProps.runtime, clock: { ...sbProps.runtime.clock, getHour: () => hour } },
      } as never));
      assert.ok(html.includes(`>${reason}<`), html);
    }
    (globalThis as Record<string, unknown>).__sbAux = { data: undefined, error: new ShownError('Scoreboard stream broke.', 'sb 503'), retry() {} };
    const failedHtml = renderToStaticMarkup(createElement(DeclaredFamilyWidget as never, sbProps as never));
    assert.match(failedHtml, /data-state="error"/, 'scoreboard stream failure renders retryable error');
    assert.match(failedHtml, /Scoreboard stream broke\./);
    assert.match(failedHtml, />Retry</);
  } finally {
    delete (globalThis as Record<string, unknown>).__sbAux;
    delete (globalThis as Record<string, unknown>).__sbFollowCheck;
    delete (globalThis as Record<string, unknown>).__datedTestGroups;
    await scoreboardVite.close();
  }
});
