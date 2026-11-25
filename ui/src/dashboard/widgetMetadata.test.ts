import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetMeta, WidgetRuntime } from './definitions';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model';
import { syntheticDashboardSource } from './source.fixture.ts';

test('widget metadata preserves world controls and content-choice title policies', async () => {
  const fixtures: Record<string, string> = {
    '/components/Popover.tsx': `export const usePopover = () => ({ trigger: null, open: true, close() {} });
      export const Popover = ({ children }) => children; export const PopoverBody = ({ children }) => children();`,
    '/dashboard/products.ts': 'export const useProduct = () => ({}); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/CountryChip.tsx': 'export default () => "COUNTRY_CHIP";',
    '/dashboard/WidgetSettingsMenu.tsx': 'export default () => null;',
    '/dashboard/widgets.tsx': 'export const WidgetBody = () => "WIDGET_BODY";',
    '/dashboard/CampaignPicker.tsx': 'export default () => "CAMPAIGN_PICKER";',
    '/dashboard/settings/CountriesSection.tsx': 'export const CountriesSection = () => "COUNTRIES_SETTINGS";',
    '/dashboard/settings/TimeSection.tsx': 'export const TimeSection = () => "TIME_SETTINGS";',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'widget-metadata-leaves', enforce: 'pre',
      resolveId: id => id === 'react-grid-layout' ? '\0metadata-grid' : undefined,
      load: id => id === '\0metadata-grid' ? `export const useContainerWidth = () => ({ width: 1200, containerRef: null, mounted: true });
        export const Responsive = ({ children }) => children;` : undefined,
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/settings/OptionFields.tsx')) return 'export const shownFields = [];\n' + _source.replace('return <>{fields.map',
          'shownFields.push({ fields, onChange }); return <>{fields.map');
        return Object.entries(fixtures).find(([suffix]) => path.endsWith(suffix))?.[1];
      },
    }] });
  try {
    const { registerWidgets } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts') as typeof import('./definitions.ts');
    const { createWidget, addGridItem } = await vite.ssrLoadModule('/ui/src/dashboard/model.ts') as typeof import('./model.ts');
    const { saveLayouts, parseLayoutLibrary } = await vite.ssrLoadModule('/ui/src/dashboard/storage.ts') as typeof import('./storage.ts');
    const { WidgetSettings } = await vite.ssrLoadModule('/ui/src/dashboard/settings/WidgetSettings.tsx') as typeof import('./settings/WidgetSettings.tsx');
    const { default: Grid } = await vite.ssrLoadModule('/ui/src/dashboard/DashboardGrid.tsx') as typeof import('./DashboardGrid.tsx');
    const { default: Picker } = await vite.ssrLoadModule('/ui/src/dashboard/WidgetPicker.tsx') as typeof import('./WidgetPicker.tsx');
    const meta: WidgetMeta = { kind: 'test-world', version: 1, title: 'World history', icon: 'history', category: 'Diplomacy',
      description: 'Recorded world outcomes.', data: true, history: true, countrySelection: false,
      countries: { max: Number.MAX_SAFE_INTEGER }, defaultCountries: { kind: 'all' },
      timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history',
      minSize: { w: 2, h: 2 }, defaultSize: { w: 4, h: 6 } };
    registerWidgets([meta, { ...meta, kind: 'test-country', title: 'Country history', countrySelection: true, category: undefined },
      { ...meta, kind: 'test-unavailable', title: 'Unavailable world', availability: { product: 'division-log-tail', params: { limit: 1 } } }]);
    const template: DashboardTemplate = { version: 1, name: 'Synthetic', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: null }], widgets: [], layouts: {} };
    const world = createWidget(meta.kind, 'main', 'world');
    const country = createWidget('test-country', 'main', 'country');
    template.widgets = [world];
    const track = { ...template.tracks[0], source: syntheticDashboardSource('source', '1', 'Synthetic campaign.battleplan'),
      branch: { id: 'branch' } } as unknown as BoundTrack;
    const runtime = { campaignTracks: template.tracks, defaultRoleId: 'primary', tracks: [track, { ...track, id: 'other' }],
      maps: { schedule() {} }, client: {}, clock: { getHour: () => 0 } } as unknown as WidgetRuntime;
    const settings = (widget: typeof world) => renderToStaticMarkup(createElement(WidgetSettings, { widget, template, runtime, onChange() {} }));
    assert.doesNotMatch(settings(world), /COUNTRIES_SETTINGS|aria-label="Countries"/);
    assert.match(settings(world), /CAMPAIGN_PICKER/);
    assert.match(settings(world), /TIME_SETTINGS/);
    assert.match(settings(country), /COUNTRIES_SETTINGS/);
    const grid = () => renderToStaticMarkup(createElement(Grid, { template, runtime, editing: false, onLayouts() {}, onApply() {}, onRemove() {} }));
    assert.doesNotMatch(grid(), /COUNTRY_CHIP/);
    assert.doesNotMatch(grid(), /Synthetic campaign/, 'a widget on the sidebar’s campaign needs no recording name under its title');
    assert.match(grid(), /World history: time, Whole campaign/);
    template.widgets = [country];
    assert.match(grid(), /COUNTRY_CHIP/);
    Object.defineProperty(globalThis, 'IntersectionObserver', { configurable: true, value: class {} });
    try {
      const waiting = grid();
      assert.match(waiting, /aria-label="Loading country history…"/); assert.doesNotMatch(waiting, /WIDGET_BODY/);
    } finally { Reflect.deleteProperty(globalThis, 'IntersectionObserver'); }
    assert.match(grid(), /WIDGET_BODY/, 'server rendering mounts the body at once');
    const picker = renderToStaticMarkup(createElement(Picker, { runtime, disabled: false, onAdd() {}, onOpen() {} }));
    assert.match(picker, /<strong>World history<\/strong><\/span>/, 'a card shows its name; the description is in the details pane');
    assert.match(picker, /<span>Diplomacy<\/span><small>\d+<\/small>/, 'a declared category is a rail entry');
    assert.match(picker, /<strong>Country history<\/strong><\/span>/);
    assert.match(picker, /<strong>Unavailable world<\/strong><small>Checking this recording…<\/small>/);
    assert.match(picker, /<button(?=[^>]*aria-label="Add Unavailable world")(?=[^>]*disabled="")[^>]*>/, 'an entry the recording may lack cannot be added yet');

    const { definition: army } = await vite.ssrLoadModule('/ui/src/widgets/army-size/definition.ts');
    const { measureChoices } = await vite.ssrLoadModule('/ui/src/widgets/army-size/armySizeMeta.ts');
    const optionFields = await vite.ssrLoadModule('/ui/src/dashboard/settings/OptionFields.tsx');
    const { scopedOptionKey } = await vite.ssrLoadModule('/ui/src/dashboard/options.ts');
    const raceMeasure = scopedOptionKey('measure', 'race');
    let available = { measures: ['divisions', 'battalions'], unknownMeasures: [] as string[] };
    const useOptionChoices = () => ({ measure: measureChoices(available) });
    registerWidgets([{ ...army, useOptionChoices }]);
    const saves: { widget: WidgetSpec; view?: boolean }[] = [];
    const settingsFor = (widget: WidgetSpec) => {
      optionFields.shownFields.length = 0;
      return renderToStaticMarkup(createElement(WidgetSettings, { widget, template, runtime,
        onChange: (widget: WidgetSpec, view?: boolean) => saves.push({ widget, view }) }));
    };
    const chooseMeasure = (value: string) => {
      const fields = optionFields.shownFields.find((row: { fields: { key: string }[] }) => row.fields.some(field => field.key === 'measure'));
      assert.ok(fields, 'the actual measure dropdown was rendered');
      fields.onChange('measure', value);
      return saves.at(-1)!;
    };
    const armyWidget = createWidget(army.kind, 'main', 'army');
    for (const title of ['Army size', 'My army', 'Number of divisions']) {
      const markup = settingsFor({ ...armyWidget, title, options: { [raceMeasure]: 'divisions' } });
      assert.match(markup, /<span class="menu-item-label">Battalions<\/span>/);
      const next = chooseMeasure('battalions');
      assert.equal(next.widget.title, title);
      assert.equal(next.widget.options?.[raceMeasure], 'battalions');
      assert.equal(next.view, true);
    }
    available = { measures: ['divisions'], unknownMeasures: [] };
    const saveCount = saves.length;
    const missingMeasure = settingsFor({ ...armyWidget, options: { [raceMeasure]: 'battalions' } });
    assert.match(missingMeasure, />Saved choice unavailable</);
    assert.match(missingMeasure, /Key: battalions/);
    assert.doesNotMatch(missingMeasure, />battalions \(not in this campaign\)</);
    assert.equal(saves.length, saveCount, 'opening another recording retains the unavailable saved measure');
    available = { measures: ['divisions', 'battalions'], unknownMeasures: [] };
    const measure = army.options.find((field: { key: string }) => field.key === 'measure');
    const { titleChoice: _optOut, ...automatic } = measure;
    registerWidgets([{ ...army, kind: 'test-auto-title', title: 'Automatic content', options: [automatic], useOptionChoices }]);
    const auto = createWidget('test-auto-title', 'main', 'automatic');
    for (const title of ['Automatic content', 'Number of divisions', 'My content']) {
      settingsFor({ ...auto, title, options: { measure: 'divisions' } });
      const next = chooseMeasure('battalions');
      assert.equal(next.widget.title, title === 'My content' ? title : 'Battalions');
    }
    settingsFor({ ...auto, title: 'Battalions', options: { measure: 'battalions' } });
    assert.equal(chooseMeasure('').widget.title, 'Automatic content');
    const { definition: ideas } = await vite.ssrLoadModule('/ui/src/widgets/idea-timeline/definition.ts');
    registerWidgets([{ ...ideas, useOptionChoices: () => ({ category: [
      { value: 'mod_visible', label: 'Visible council' }, { value: 'mod_empty', label: '' },
    ] }) }]);
    const timeline = createWidget(ideas.kind, 'main', 'ideas');
    for (const title of [ideas.title, 'Visible council', 'My council']) {
      const markup = settingsFor({ ...timeline, title, options: { category: 'mod_visible' } });
      assert.match(markup, /<span class="menu-item-label"><\/span>/, 'the recorded empty category label remains empty');
      const fields = optionFields.shownFields.find((row: { fields: { key: string }[] }) => row.fields.some(field => field.key === 'category'));
      assert.ok(fields, 'the actual category dropdown was rendered');
      fields.onChange('category', 'mod_empty');
      const next = saves.at(-1)!;
      assert.equal(next.widget.title, title === 'My council' ? title : '');
      assert.equal(next.widget.options?.category, 'mod_empty');
      assert.equal(next.view, true);
      let stored = '';
      const storage = { getItem: () => stored, setItem: (_key: string, value: string) => { stored = value; } };
      assert.equal(saveLayouts({ version: 1, activeId: 'categories', layouts: [{ id: 'categories', template: {
        ...template, widgets: [next.widget], layouts: addGridItem({}, next.widget),
      } }] }, storage), null, 'an actual category settings change remains immediately savable');
      const restored = parseLayoutLibrary(JSON.parse(stored));
      assert.ok(restored);
      assert.equal(restored.layouts[0].template.widgets[0].title, next.widget.title);
      assert.equal(restored.layouts[0].template.widgets[0].options?.category, 'mod_empty');
    }
    settingsFor({ ...timeline, title: '', options: { category: 'mod_empty' } });
    const emptyCategoryFields = optionFields.shownFields.find((row: { fields: { key: string }[] }) => row.fields.some(field => field.key === 'category'));
    assert.ok(emptyCategoryFields);
    emptyCategoryFields.onChange('category', 'mod_visible');
    assert.equal(saves.at(-1)!.widget.title, 'Visible council', 'an empty authored title still follows its selected content choice');
  } finally { await vite.close(); }
});
