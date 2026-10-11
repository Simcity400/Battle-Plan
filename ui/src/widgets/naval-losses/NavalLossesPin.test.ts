import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, isValidElement } from 'react';
import type { ComponentProps, ComponentType, ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type DrillHost from '../../charts/drill/DrillHost';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type CountryPicker from '../../components/CountryPicker';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountrySelection, DashboardTemplate, WidgetSpec } from '../../dashboard/model';
import type { CountriesSection } from '../../dashboard/settings/CountriesSection';
import type { WidgetSettings } from '../../dashboard/settings/WidgetSettings';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { CountryGroupReply, NavalLossesReply } from '../../generated/dashboard';

test('actual Naval Pin keeps its dated native actor and mounted callback lifetime', async context => {
  const identity = (firstHour: number, lastHour: number, slot: number, countryId: number) => ({ firstHour, lastHour, slot, countryId });
  const history = (tag: string, identities: NonNullable<NavalLossesReply['series'][number]['identities']>, ships: number, convoys: number) => ({
    tag, membership: null, identities, segments: identities.map(({ firstHour, lastHour }) => ({ firstHour, lastHour })),
    runs: identities.map(({ firstHour, lastHour, slot }) => ({ firstHour, lastHour, startsSegment: true,
      ships: slot === 1 ? 4 : slot === 3 ? 900 : ships, convoys: slot === 1 ? -2 : slot === 3 ? 0 : convoys })),
  });
  const reply: NavalLossesReply = { branchId: '1', available: true, firstRead: 100, shipFirstRead: 100, convoyFirstRead: 100,
    shipLastRead: 100, convoyLastRead: 100, shipReads: [100], convoyReads: [100], warnings: [], series: [
      history('mod A', [identity(104, 112, 1, 11)], 4, -2),
      history('old B', [identity(100, 103, 2, 22)], 40, -3),
      history('reused alias', [identity(100, 103, 1, 11), identity(104, 110, 2, 22), identity(111, 112, 3, 33)], 40, -3),
      history('new B', [identity(111, 112, 2, 22)], 40, -3),
    ] };
  const groups: CountryGroupReply = { branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [],
    unobserved: { humans: [], factions: [], wars: [] }, factions: [], wars: [],
    humans: [{ tag: 'mod A', firstHour: 104, lastHour: 112 }, { tag: 'new B', firstHour: 111, lastHour: 112 }],
    countryIdentities: reply.series.map(({ tag, identities }) => ({ tag, identities: identities! })) };
  type Effect = { setup: () => void | (() => void); cleanup?: () => void };
  const state = { reply, groups, hour: 108, hooks: [] as unknown[], cursor: 0, dirty: false, effects: [] as (() => void)[],
    main: undefined as CountryRaceViewProps | undefined, detail: undefined as CountryRaceViewProps | undefined,
    host: undefined as ComponentProps<typeof DrillHost> | undefined, picker: undefined as ComponentProps<typeof CountryPicker> | undefined,
    settingsCountries: undefined as ComponentProps<typeof CountriesSection> | undefined,
    labels: {} as Record<string, ReturnType<CountryLabel>>,
    labelRequests: [] as { wanted: { tag: string; hour: number }[]; active: boolean }[],
    countrySlot: {} as HTMLElement, portals: [] as { child: ReactNode; target: HTMLElement }[],
    added: [] as WidgetSpec[] };
  const globals = globalThis as typeof globalThis & { __navalPinLifecycle?: typeof state };
  globals.__navalPinLifecycle = state;
  // Execute the actual adapter's effect setup/cleanup and retain hook cells only
  // for that mount. The shared reducer, holder projection and emitted spec are real.
  const hooks = `const cell = init => { const state = globalThis.__navalPinLifecycle, index = state.cursor++; return state.hooks[index] ??= init(); };
    const changed = (before, after) => before === null || before.length !== after.length || before.some((dep, i) => !Object.is(dep, after[i]));
    export const useEffect = (setup, deps) => { const saved = cell(() => ({ deps: null, setup, cleanup: undefined })); saved.setup = setup;
      if (changed(saved.deps, deps)) { saved.deps = deps; globalThis.__navalPinLifecycle.effects.push(() => { saved.cleanup?.(); saved.cleanup = setup(); }); } };
    export const useRef = initial => cell(() => ({ current: initial }));
    export const useMemo = (make, deps) => { const saved = cell(() => ({ deps, value: make() }));
      if (changed(saved.deps, deps)) { saved.deps = deps; saved.value = make(); } return saved.value; };
    export const useCallback = (callback, deps) => useMemo(() => callback, deps);
    export const useState = initial => { const saved = cell(() => ({ value: typeof initial === 'function' ? initial() : initial }));
      return [saved.value, next => { const value = typeof next === 'function' ? next(saved.value) : next;
        if (!Object.is(value, saved.value)) { saved.value = value; globalThis.__navalPinLifecycle.dirty = true; } }]; };
    export const useReducer = (reduce, initial) => { const saved = cell(() => ({ value: initial }));
      return [saved.value, action => { const value = reduce(saved.value, action);
        if (!Object.is(value, saved.value)) { saved.value = value; globalThis.__navalPinLifecycle.dirty = true; } }]; };`;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = (_client, request) => ({ data: request ? globalThis.__navalPinLifecycle.reply : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__navalPinLifecycle.groups });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted, active) => {
      const state = globalThis.__navalPinLifecycle; state.labelRequests.push({ wanted, active });
      return { labelAt: hour => tag => state.labels[JSON.stringify([hour, tag])] ?? { name: tag + '@' + hour }, revision: 0 }; };`,
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123456", "#654321"] });',
    '/charts/race/CountryRaceView.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__navalPinLifecycle[props.view === 'mix' ? 'detail' : 'main'] = props; return createElement('div', null, props.beforeChart); };`,
    '/components/CountryPicker.tsx': 'export default props => { globalThis.__navalPinLifecycle.picker = props; return null; };',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__navalPinLifecycle.host = props; return createElement('div', null, props.main, props.open ? props.children : null); };`,
    '/dashboard/settings/CountriesSection.tsx': 'export const CountriesSection = props => { globalThis.__navalPinLifecycle.settingsCountries = props; return null; };',
    '/dashboard/settings/OptionFields.tsx': 'export const OptionFields = () => null;',
    '/dashboard/settings/TimeSection.tsx': 'export const TimeSection = () => null;',
    '/dashboard/FactionColours.tsx': 'export const FactionColoursEditor = () => null;',
    '/dashboard/CampaignPicker.tsx': 'export default () => null;',
    '/dashboard/WidgetSettingsMenu.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'naval-pin-lifecycle-fixture', enforce: 'pre',
      resolveId(id) { if (id === 'naval-pin-hooks') return '\0naval-pin-hooks'; if (id === 'naval-pin-portals') return '\0naval-pin-portals'; if (id === 'react-grid-layout') return '\0naval-pin-grid'; },
      load(id) { if (id === '\0naval-pin-hooks') return hooks; if (id === '\0naval-pin-portals') return 'export const createPortal = (child, target) => { globalThis.__navalPinLifecycle.portals.push({ child, target }); return null; };';
        if (id === '\0naval-pin-grid') return `export const useContainerWidth = () => ({ width: 1200, containerRef: null, mounted: true });
        export const Responsive = ({ children }) => children;`; },
      transform(source, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/naval-losses/NavalLossesWidget.tsx')) return source.replace("from 'react'", "from 'naval-pin-hooks'").replace("from 'react-dom'", "from 'naval-pin-portals'");
        if (path.endsWith('/dashboard/DashboardGrid.tsx')) return source.replace('const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(null);',
          'const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(globalThis.__navalPinLifecycle.countrySlot);')
          .replace('onDiagnostics={setDiagnostics}', 'onDiagnostics={() => {}}');
      },
    }] });
  const unmount = () => {
    for (const cell of state.hooks) if (cell && typeof cell === 'object' && 'cleanup' in cell) (cell as Effect).cleanup?.();
    state.hooks = []; state.effects = []; state.portals = []; state.dirty = false;
  };
  try {
    const registry = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    registry.registerWidgets([{ ...(await vite.ssrLoadModule('/ui/src/widgets/naval-losses/definition.ts')).definition,
      ...(await vite.ssrLoadModule('/ui/src/widgets/naval-losses/index.ts')).default }]);
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/naval-losses/NavalLossesWidget.tsx')).default as ComponentType<WidgetProps>;
    const Settings = (await vite.ssrLoadModule('/ui/src/dashboard/settings/WidgetSettings.tsx')).WidgetSettings as typeof WidgetSettings;
    const Grid = (await vite.ssrLoadModule('/ui/src/dashboard/DashboardGrid.tsx')).default as typeof import('../../dashboard/DashboardGrid').default;
    const original: WidgetSpec = { id: 'naval', kind: 'naval-losses', configVersion: 1, title: 'Naval losses', trackIds: ['track'],
      countries: { kind: 'tags', tags: ['mod A', 'old B'] }, time: { mode: 'campaign' }, options: { view: 'lines' } };
    const props = { widget: original, runtime: { tracks: [{ id: 'track', axisOffset: 100,
      source: { id: 'synthetic', path: 'C:/captures/synthetic.battleplan', revision: '1', metadata: { countries: [{ id: 33, tag: 'reused alias', name: 'Replacement C',
        observedBranchId: '1', observedHour: 112 }], branches: [{ id: '1', firstHour: 100, lastHour: 112 }] } }, branch: { id: '1', firstHour: 100, lastHour: 112 } }],
      client: {}, clock: { getHour: () => state.hour - 100, getSnapshot: () => ({ playing: false }) },
      maps: { schedule() {} }, campaignTracks: [], defaultRoleId: null,
      addWidgetBelow: (_id: string, pinned: WidgetSpec) => state.added.push(pinned) }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const commit = () => {
      for (let pass = 0; pass < 12; pass++) {
        state.cursor = 0; state.dirty = false; state.main = undefined; state.detail = undefined; state.host = undefined; state.picker = undefined;
        renderToStaticMarkup(createElement(Widget, props));
        for (const effect of state.effects.splice(0)) effect();
        if (!state.dirty && !state.effects.length) return;
      }
      assert.fail('the Naval adapter did not settle its mounted effects');
    };
    const assertB = (tag: string, hour: number) => {
      assert.ok(state.detail?.mix, 'a fresh mount restores one native actor without asking for another choice');
      assert.equal(state.detail!.mix!.tag, tag);
      assert.equal(state.detail!.moment, hour);
      assert.equal(state.detail!.state, undefined);
      assert.equal(state.picker!.selectedTag, tag, 'restoration opens the saved native actor instead of a multi-holder chooser');
      assert.deepEqual(new Set(state.detail!.mix!.bands![0].map(run => run.value)), new Set([40]));
      assert.deepEqual(new Set(state.detail!.mix!.bands![1].map(run => run.value)), new Set([0]), 'signed corrections retain no positive geometry');
      assert.deepEqual(state.detail!.tooltipTags!(102, ['reused alias']), ['old B']);
      const tip = [...state.detail!.tooltip(tag, hour, tag, new Map(), () => alias => ({ name: alias }))!][0];
      assert.equal(tip.rows!.find(row => row.label === 'Ships')!.value, '40');
      assert.equal(tip.rows!.find(row => row.label === 'Convoys')!.value, '-3');
      assert.equal(tip.rows!.find(row => row.label === 'Lost')!.value, '37');
    };
    const unmountWithHeldCallbacks = () => {
      const pin = state.host!.onPin!, follow = state.host!.onFollow, close = state.host!.onClose,
        pick = state.detail!.onPick!, choose = state.picker!.onChange;
      const count = state.added.length;
      unmount(); // No inactive render; the same mounted active callbacks remain held.
      pin(); follow(); close(); pick(102); choose('mod A');
      assert.equal(state.added.length, count, 'an unmounted available Pin callback cannot create a widget');
      assert.equal(state.dirty, false, 'no held callback changes the abandoned mount');
      return pin;
    };
    await context.test('emitted Pin and Follow specs restore only the chosen native holder after fresh mount', () => {
      commit();
      state.main!.onPick!(108, 'reused alias', ['reused alias']); commit();
      state.detail!.onPick!(108); commit(); state.host!.onPin!();
      const picked = JSON.parse(JSON.stringify(state.added.at(-1))) as WidgetSpec;
      assert.deepEqual(picked.countries, { kind: 'tags', tags: ['reused alias'] });
      assert.deepEqual(picked.time, { mode: 'pin', windows: { track: { firstOffset: 8, lastOffset: 8 } } });
      unmount(); props.widget = picked; state.hour = 112; commit(); assertB('reused alias', 108);
      assert.equal(picked.options!.drillOriginOffset, 8);
      assert.equal(picked.options!.drillOriginTag, 'reused:20:alias');
      unmount(); props.widget = original; state.hour = 108; commit();
      state.main!.onPick!(108, 'reused alias', ['reused alias']); commit(); state.host!.onPin!();
      const following = JSON.parse(JSON.stringify(state.added.at(-1))) as WidgetSpec;
      assert.deepEqual(following.time, { mode: 'follow' });
      unmount(); props.widget = following; state.hour = 112; commit(); assertB('new B', 112);
      assert.equal(following.options!.drillOriginOffset, 8);
      assert.equal(following.options!.drillOriginTag, 'reused:20:alias');
      state.hour = 102; commit(); assertB('old B', 102);
      for (const offset of [-1, 0.5, 8 + 1e-10, 99, 'invalid']) {
        unmount(); props.widget = { ...following, options: { ...following.options, drillOriginOffset: offset } }; commit();
        assert.equal(state.detail!.mix, null);
        assert.match(renderToStaticMarkup(state.detail!.state), /Saved country selection unavailable/);
        state.picker!.onChange('reused alias'); commit(); assert.equal(state.detail!.mix, null, 'invalid origins do not fall back to a current tag holder');
      }
      for (const identities of [undefined, [], [identity(100, 103, 1, 11), identity(109, 110, 2, 22), identity(111, 112, 3, 33)]]) {
        unmount(); props.widget = following;
        state.reply = { ...reply, series: reply.series.map(series => series.tag === 'reused alias' ? { ...series, identities } : series) }; commit();
        assert.equal(state.detail!.mix, null);
        assert.match(renderToStaticMarkup(state.detail!.state), /Saved country selection unavailable/,
          'absent, empty and origin-gap identity metadata cannot be replaced by counts or later holders');
      }
    });
    await context.test('adapter effect cleanup rejects held callbacks while setup replay and a new mount stay usable', () => {
      unmount(); state.reply = reply; props.widget = original; state.hour = 108; commit();
      // StrictMode's cleanup/setup replay republishes this same mounted instance.
      const mountedEffects = state.hooks.filter((cell): cell is Effect => !!cell && typeof cell === 'object' && 'setup' in cell);
      for (const cell of mountedEffects) cell.cleanup?.();
      for (const cell of mountedEffects) { const cleanup = cell.setup(); cell.cleanup = typeof cleanup === 'function' ? cleanup : undefined; }
      commit(); state.main!.onPick!(108, 'reused alias', ['reused alias']); commit();
      assert.equal(state.detail!.mix!.tag, 'reused alias', 'effect setup replay re-admits the live mounted instance');
      const abandonedPin = unmountWithHeldCallbacks(), count = state.added.length;
      props.widget = original; commit(); abandonedPin();
      assert.equal(state.added.length, count, 'fresh mount never re-admits the old instance callback');
      state.main!.onPick!(108, 'reused alias', ['reused alias']); commit(); state.host!.onPin!();
      assert.equal(state.added.length, count + 1, 'the replacement mount can still pin its own actor');
    });
    await context.test('actual Countries settings preserve unchanged origins and release changed selections on update and fresh mount', () => {
      unmount(); state.reply = reply; props.widget = original; state.hour = 108; commit();
      state.main!.onPick!(108, 'reused alias', ['reused alias']); commit(); state.host!.onPin!();
      const saved = JSON.parse(JSON.stringify(state.added.at(-1))) as WidgetSpec;
      const template: DashboardTemplate = { version: 1, name: 'Naval settings', alignment: 'calendar',
        roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
        tracks: [{ id: 'track', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: null }],
        widgets: [saved], layouts: {} };
      const selections: CountrySelection[] = [saved.countries, { kind: 'tags', tags: ['new B'] },
        { kind: 'tags', tags: ['mod A', 'new B'] }, { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }];
      for (const countries of selections) {
        unmount(); props.widget = saved; state.hour = 112; commit(); assertB('new B', 112);
        const changes: WidgetSpec[] = [];
        state.settingsCountries = undefined;
        renderToStaticMarkup(createElement(Settings, { widget: saved, template, runtime: props.runtime,
          onChange: (next, view) => { assert.equal(view, true); changes.push(next); } }));
        const settingsCountries = state.settingsCountries as ComponentProps<typeof CountriesSection> | undefined;
        assert.ok(settingsCountries, 'the actual settings panel supplies its Countries callback');
        settingsCountries.onChange(countries);
        assert.equal(changes.length, 1);
        const changed = changes[0];
        assert.ok(changed);
        assert.deepEqual(changed.countries, countries);
        assert.deepEqual(changed.options, saved.options, 'the actual settings callback retains the old hidden origin witness');
        const assertSelection = () => {
          if (countries.kind === 'group' || countries.kind === 'tags' && countries.tags.length > 1) {
            assert.equal(state.detail, undefined, 'a changed multi-country selection opens the normal chooser');
            assert.equal(state.picker!.selectedTag, null);
            assert.deepEqual([...state.picker!.allowedTags!].sort(), ['mod A', 'new B']);
            state.picker!.onChange('new B'); commit();
          }
          assertB('new B', 112);
        };
        props.widget = changed; commit(); assertSelection();
        unmount(); props.widget = JSON.parse(JSON.stringify(changed)) as WidgetSpec; commit(); assertSelection();
        if (countries.kind === 'tags' && countries.tags.length === 1) {
          state.hour = 102; commit(); assertB('old B', 102);
        }
      }
    });
    await context.test('registered countrySlot follows the actual pinned actor and displayed date without layout writes', () => {
      unmount(); state.reply = reply; props.widget = original; state.hour = 108; commit();
      state.main!.onPick!(108, 'reused alias', ['reused alias']); commit(); state.host!.onPin!();
      const following = JSON.parse(JSON.stringify(state.added.at(-1))) as WidgetSpec;
      const template: DashboardTemplate = { version: 1, name: 'Naval header', alignment: 'calendar', roles: [], tracks: [], widgets: [following], layouts: {} };
      const changes: WidgetSpec[] = [];
      const frame = () => {
        for (let pass = 0; pass < 12; pass++) {
          state.cursor = 0; state.dirty = false; state.main = undefined; state.detail = undefined; state.host = undefined; state.picker = undefined;
          state.portals = []; state.labelRequests = [];
          const html = renderToStaticMarkup(createElement(Grid, { template, runtime: props.runtime,
            editing: false, onLayouts() {}, onApply: next => changes.push(next), onRemove() {} }));
          for (const effect of state.effects.splice(0)) effect();
          if (!state.dirty && !state.effects.length) return html;
        }
        return assert.fail('the registered frame body did not settle its mounted effects');
      };
      for (const [tag, hour, name, flag] of [
        ['new B', 112, 'Native B at 112', '/B112.svg'], ['old B', 102, 'Native B at 102', '/B102.svg'],
        ['reused alias', 108, 'Native B at 108', '/B108.svg'], ['reused alias', 112, 'Native C at 112', '/C112.svg'],
        ['mod A', 112, 'Native A at 112', '/A112.svg'],
      ] as const) state.labels[JSON.stringify([hour, tag])] = { name, flag, known: true, flagPending: false };
      const header = () => {
        assert.equal(state.portals.length, 1, 'the actual registered body owns one countrySlot portal');
        assert.equal(state.portals[0].target, state.countrySlot, 'the frame supplies the target in its country-chip position');
        return renderToStaticMarkup(state.portals[0].child);
      };
      const assertHeader = (tag: string, hour: number, name: string, flag: string) => {
        const html = header();
        assert.ok(html.includes(name)); assert.ok(html.includes('src="' + flag + '"'));
        assert.ok(html.includes('As of ' + formatDateTime(hour)), 'header date is the actual displayed drill date');
        assert.doesNotMatch(html, /Replacement C/);
        assert.deepEqual(state.labelRequests.at(-1), { wanted: [{ tag, hour }], active: true });
      };
      unmount(); state.hour = 112; frame(); assertB('new B', 112); assertHeader('new B', 112, 'Native B at 112', '/B112.svg');
      state.picker!.onChange('reused alias'); frame();
      assert.equal(state.detail!.mix!.tag, 'reused alias'); assert.equal(state.detail!.moment, 112);
      assert.deepEqual(new Set(state.detail!.mix!.bands![0].map(run => run.value)), new Set([900]));
      assertHeader('reused alias', 112, 'Native C at 112', '/C112.svg');
      assert.equal(following.options!.drillOriginOffset, 8, 'local replacement choice does not rewrite the saved restoration anchor');
      state.detail!.onPick!(108); frame();
      assert.ok(state.detail!.state, 'C remains selected where its history has no recorded native identity');
      assert.match(header(), /unavailable/); assert.doesNotMatch(header(), /Native B|B108.svg/);
      assert.deepEqual(state.labelRequests.at(-1), { wanted: [], active: false }, 'a historical gap cannot retarget the saved B holder');
      const recoveryButton = (state.detail!.beforeChart as ReactElement<{ children: ReactNode[] }>).props.children[0];
      assert.ok(isValidElement<{ onClick: () => void }>(recoveryButton)); recoveryButton.props.onClick(); frame();
      assertHeader('reused alias', 112, 'Native C at 112', '/C112.svg');
      state.picker!.onChange('mod A'); frame();
      assert.deepEqual(new Set(state.detail!.mix!.bands![0].map(run => run.value)), new Set([4]));
      assertHeader('mod A', 112, 'Native A at 112', '/A112.svg');
      unmount(); props.widget = original; state.hour = 108; commit();
      state.main!.onPick!(108, 'reused alias', ['reused alias']); commit(); state.detail!.onPick!(108); commit(); state.host!.onPin!();
      const pinned = JSON.parse(JSON.stringify(state.added.at(-1))) as WidgetSpec;
      const savedTime = { mode: 'pin', windows: { track: { firstOffset: 8, lastOffset: 8 } } };
      unmount(); template.widgets = [pinned]; state.hour = 112; frame(); assertB('reused alias', 108);
      assertHeader('reused alias', 108, 'Native B at 108', '/B108.svg');
      state.detail!.onPick!(112); frame(); assertB('new B', 112); assertHeader('new B', 112, 'Native B at 112', '/B112.svg');
      assert.deepEqual(pinned.time, savedTime, 'local historical pick keeps the original saved Pin at 108');
      const followButton = (state.detail!.beforeChart as ReactElement<{ children: ReactNode[] }>).props.children[0];
      assert.ok(isValidElement<{ onClick: () => void }>(followButton)); assert.equal(followButton.type, 'button');
      state.hour = 102; followButton.props.onClick(); frame(); assertB('old B', 102); assertHeader('old B', 102, 'Native B at 102', '/B102.svg');
      assert.deepEqual(pinned.time, savedTime, 'local Follow uses its displayed date without rewriting WidgetSpec.time');
      state.hour = 112; frame(); assertB('new B', 112); assertHeader('new B', 112, 'Native B at 112', '/B112.svg');
      const labelKey = JSON.stringify([112, 'new B']);
      state.labels[labelKey] = { name: '', known: true, flagPending: true }; frame();
      assert.match(header(), /Flag loading/); assert.doesNotMatch(header(), /Selected country unavailable|Replacement C/);
      state.labels[labelKey] = { name: '', known: true, flagPending: false, flagReason: 'Displayed flag was not recorded' }; frame();
      assert.match(header(), /Flag unavailable/);
      state.labels[labelKey] = { name: 'Unproved name', known: false, nameFailed: true, flag: '/B112.svg', flagPending: false }; frame();
      assert.match(header(), /Unavailable/); assert.doesNotMatch(header(), /Unproved name/);
      assert.ok(header().includes('src="/B112.svg"'), 'the authenticated current flag remains independent of a failed name');
      assert.match(header(), /Name unavailable/);
      state.labels[labelKey] = { name: 'Unproved name', known: false, nameFailed: true, flagPending: false,
        flagReason: 'Exact displayed flag refusal' }; frame();
      assert.match(header(), /Flag unavailable/);
      assert.match(header(), /Name unavailable/);
      state.labels[labelKey] = { name: 'Unproved name', known: false, nameFailed: true, flagPending: true }; frame();
      assert.match(header(), /Flag loading/); assert.doesNotMatch(header(), /Flag not available: The country name request failed/);
      state.labels[labelKey] = { name: 'Native B at 112', known: true, flag: '/B112.svg', flagPending: false };
      state.reply = { ...reply, series: reply.series.map(series => series.tag === 'new B' ? { ...series, identities: [] } : series) }; frame();
      assert.match(header(), /unavailable/); assert.doesNotMatch(header(), /Native B|B112.svg/);
      assert.deepEqual(state.labelRequests.at(-1), { wanted: [], active: false }, 'a displayed identity gap never requests another holder’s looks');
      state.reply = { ...reply, series: [...reply.series, history('conflicting B', [identity(111, 112, 2, 22)], 40, -3)] }; frame();
      assert.match(header(), /unavailable/); assert.deepEqual(state.labelRequests.at(-1), { wanted: [], active: false });
      assert.ok(header().includes('As of ' + formatDateTime(112)), 'a known displayed date remains explicit when identity is refused');
      state.reply = reply; state.labels = {}; unmount(); template.widgets = [original]; state.hour = 112;
      assert.match(frame(), /Country name unavailable/); assert.equal(state.portals.length, 0);
      state.main!.onPick!(108, 'reused alias', ['reused alias']); assert.match(frame(), /Country name unavailable/);
      assert.equal(state.portals.length, 0, 'ordinary main/comparison headers keep their country roots during an inline drill');
      assert.deepEqual(changes, [], 'local choices, history and Follow never write layout preferences');
    });
  } finally { unmount(); await vite.close(); delete globals.__navalPinLifecycle; }
});
