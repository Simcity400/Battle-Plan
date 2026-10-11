import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType, ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetCountryChipProps, WidgetProps } from '../../dashboard/definitions';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { WidgetSpec } from '../../dashboard/model';
import type { CountryRaceViewProps } from './CountryRaceView';
import type { RaceChartProps } from './RaceChart';
import type { TimeChartProps } from '../TimeChart';
import type { NativePartsRaceReply } from './partsRaceDrill';
import { tokensFromValues } from '../tokens.ts';
import { plotFrame } from '../frame.ts';

const series = (tag: string, slot: number, countryId: number, firstHour: number, lastHour: number, total: number) => ({ tag,
  identities: [{ firstHour, lastHour, slot, countryId }], membership: null, segments: [{ firstHour, lastHour }],
  runs: [{ firstHour, lastHour, startsSegment: true, total, values: [total], referenceTotal: 100 }] });
const old = series('OLD', 7, 101, 0, 49, 5), reuse = series('OLD', 8, 102, 100, 180, 99);
old.identities.push(...reuse.identities); old.segments.push(...reuse.segments); old.runs.push(...reuse.runs);
const renamed = series('NEW', 7, 101, 50, 180, 7);
renamed.identities = [{ firstHour: 50, lastHour: 89, slot: 7, countryId: 101 }, { firstHour: 91, lastHour: 180, slot: 7, countryId: 101 }];
const reply: NativePartsRaceReply = { branchId: '1', warnings: [], parts: [{ key: 'mod type', name: 'Recorded type', unresolved: false }], series: [old, renamed] };

test('registered force breakdowns date their native headers and follow an independent clock with owned controls', async () => {
  const state = { reply: reply as NativePartsRaceReply | undefined, hour: 113, slots: [] as unknown[], cursor: 0,
    effects: [] as (() => unknown)[], portals: [] as ReactElement<{ label: ReturnType<CountryLabel> | null; hour: number | null }>[], views: [] as CountryRaceViewProps[],
    charts: [] as TimeChartProps[], races: [] as RaceChartProps[], lists: [] as { onPick?: (id: string) => void }[],
    subscriptions: [] as boolean[], pins: [] as WidgetSpec[], sought: [] as number[], menu: [] as { run: () => void }[],
    demands: [] as { tag: string; hour: number }[], host: null as { onFollow: () => void; onPause?: () => void; onPin?: () => void } | null };
  const root = globalThis as typeof globalThis & { __partsIdentity?: typeof state; __partsHooks?: {
    useMemo: (factory: () => unknown, deps: readonly unknown[]) => unknown;
    useRef: (value: unknown) => unknown; useState: (value: unknown) => unknown;
    useReducer: (reducer: (value: unknown, action: unknown) => unknown, value: unknown) => unknown;
    useEffect: (effect: () => unknown, deps: readonly unknown[]) => unknown;
  } };
  root.__partsIdentity = state;
  root.__partsHooks = {
    useMemo(factory: () => unknown, deps: readonly unknown[]) {
      const index = state.cursor++, previous = state.slots[index] as { deps: readonly unknown[]; value: unknown } | undefined;
      if (previous && previous.deps.length === deps.length && previous.deps.every((value, i) => Object.is(value, deps[i]))) return previous.value;
      const value = factory(); state.slots[index] = { deps: [...deps], value }; return value;
    },
    useRef(value: unknown) { const index = state.cursor++; return state.slots[index] ??= { current: value }; },
    useState(value: unknown) {
      const index = state.cursor++;
      if (!(index in state.slots)) state.slots[index] = value;
      return [state.slots[index], (next: unknown) => { state.slots[index] = next; }];
    },
    useReducer(reducer: (value: unknown, action: unknown) => unknown, value: unknown) {
      const index = state.cursor++;
      if (!(index in state.slots)) state.slots[index] = value;
      return [state.slots[index], (action: unknown) => { state.slots[index] = reducer(state.slots[index], action); }];
    },
    useEffect(effect: () => unknown, deps: readonly unknown[]) {
      root.__partsHooks!.useMemo(() => { state.effects.push(effect); }, deps);
    },
  };
  const replacements: Record<string, string> = {
    '/dashboard/model.ts': `export const resolveWindow = (widget, track, _tracks, hour) => {
      const window = widget.time.windows?.[track.id];
      if (widget.time.mode === 'pin' && !window) return null;
      return { firstHour: widget.time.mode === 'range' ? track.branch.firstHour + widget.time.firstOffset
        : window ? track.branch.firstHour + window.firstOffset : track.branch.firstHour,
        lastHour: widget.time.mode === 'follow' ? hour + track.axisOffset : widget.time.mode === 'range'
          ? track.branch.firstHour + widget.time.lastOffset : window ? track.branch.firstHour + window.lastOffset : track.branch.lastHour }; };
      export const seekPositionFor = (track, hour) => hour >= track.branch.firstHour && hour <= track.branch.lastHour ? hour - track.axisOffset : null;
      export const createWidget = (kind, id) => ({ kind, id: 'new pin', configVersion: 1, trackIds: [id], options: {} });`,
    '/dashboard/playback.ts': `export const usePlaybackHour = (_clock, on) => { globalThis.__partsIdentity.subscriptions.push(on); };
      export const usePlaybackSpeed = () => 24;`,
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request?.product === 'country-groups' ? undefined
      : request ? globalThis.__partsIdentity.reply : undefined, retry() {} });`,
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({ name: tag => tag, color: () => [50,100,150], flag: () => undefined, revision: 0 });`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted, active) => {
      if (active) globalThis.__partsIdentity.demands.push(...wanted);
      return { labelAt: hour => tag => ({ known: true, name: tag + ' at ' + hour, flag: 'flag:' + tag + ':' + hour }), revision: 0 }; };`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#777', 'sans-serif'))});
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': `export const useContextMenu = () => ({ open: (_x, _y, items) => { globalThis.__partsIdentity.menu = items; }, menu: null });`,
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': `export default props => { globalThis.__partsIdentity.host = props; return props.open ? props.children : props.main; };`,
    '/charts/drill/DrillParts.tsx': `export default props => { globalThis.__partsIdentity.lists.push(props); return null; };`,
    '/charts/TimeChart.tsx': `export default props => { globalThis.__partsIdentity.charts.push(props); return null; };`,
    '/charts/race/RaceChart.tsx': `export default props => { globalThis.__partsIdentity.races.push(props); return null; };`,
    '/charts/Legend.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'parts-native-inputs', enforce: 'pre',
      resolveId(id) { if (id === 'parts-native-portals') return '\0parts-native-portals'; },
      load(id) { if (id === '\0parts-native-portals') return 'export const createPortal = (child) => { globalThis.__partsIdentity.portals.push(child); return null; };'; },
      transform(code, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/race/PartsRaceView.tsx') || path.endsWith('/race/CountryPartsDrill.tsx')) return code.replace("from 'react-dom'", "from 'parts-native-portals'")
          .replace(/\b(useMemo|useRef|useState|useReducer|useEffect)(?:<[\s\S]*?>)?\(/g, 'globalThis.__partsHooks.$1(');
        if (path.endsWith('/race/CountryRaceView.tsx')) return code.replace('const tokens = useChartTokens();',
          'globalThis.__partsIdentity.views.push({ mix, moment, extent, playhead, beforeChart, onPick }); const tokens = useChartTokens();');
      },
    }] });
  try {
    const RaceView = (await vite.ssrLoadModule('/ui/src/charts/race/CountryRaceView.tsx')).default as ComponentType<CountryRaceViewProps>;
    const { default: CountryPartsDrill } = await vite.ssrLoadModule('/ui/src/charts/race/CountryPartsDrill.tsx') as typeof import('./CountryPartsDrill.tsx');
    const { default: FollowControl } = await vite.ssrLoadModule('/ui/src/charts/drill/FollowControl.tsx') as typeof import('../drill/FollowControl.tsx');
    const { partsRaceDrillData, nativePartsRaceNumbers } = await vite.ssrLoadModule('/ui/src/charts/race/partsRaceDrill.ts') as typeof import('./partsRaceDrill.ts');
    const { partsRaceModel } = await vite.ssrLoadModule('/ui/src/dashboard/partsRace.ts') as typeof import('../../dashboard/partsRace.ts');
    for (const kind of ['air-force-size', 'navy-size']) {
      const { default: registered } = await vite.ssrLoadModule(`/ui/src/widgets/${kind}/index.ts`) as { default: { render: ComponentType<WidgetProps>; countryChip: ComponentType<WidgetCountryChipProps> } };
      assert.match(renderToStaticMarkup(createElement(registered.countryChip, { widget: { options: { show: 'breakdown' } }, onSlot() {} } as unknown as WidgetCountryChipProps)), /country-slot/);
      const track = { id: 'track', axisOffset: 7, source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [], branches: [] } },
        branch: { id: '1', firstHour: 0, lastHour: 180, gaps: [] } };
      const props = { widget: { id: 'widget', kind, configVersion: 1, title: 'Force', trackIds: ['track'], countries: { kind: 'tags', tags: ['OLD'] },
        time: { mode: 'pin', windows: { track: { firstOffset: 100, lastOffset: 100 } } },
        options: { show: 'breakdown', drillOriginHour: 20, holder: JSON.stringify(['campaign', '1', 'OLD', 7, 101]) } },
        runtime: { tracks: [track], client: {}, clock: { getHour: () => state.hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) },
          requestSeek: (_id: string, hour: number) => state.sought.push(hour), addWidgetBelow: (_id: string, pin: WidgetSpec) => state.pins.push(pin) },
        active: true, countrySlot: {} as HTMLElement, onDiagnostics() {} } as unknown as WidgetProps;
      const render = () => {
        state.cursor = 0; state.effects = []; state.views = []; state.charts = []; state.portals = []; state.demands = []; state.races = []; state.lists = [];
        renderToStaticMarkup(createElement(registered.render, props));
        for (const effect of state.effects) effect();
        return state.views[0];
      };
      const button = (node: unknown): (() => void) | null => {
        if (!node || typeof node !== 'object') return null;
        if (Array.isArray(node)) return node.map(button).find(Boolean) ?? null;
        const element = node as ReactElement<{ children?: unknown; onClick?: () => void }>;
        if (typeof element.type === 'function' && element.type.name === 'FollowControl') return FollowControl(element.props as Parameters<typeof FollowControl>[0]).props.onClick;
        if (element.type === 'button' && element.props.children === 'Follow') return element.props.onClick ?? null;
        return button(element.props?.children);
      };
      state.slots = []; state.reply = reply; state.hour = 113;
      let body = render();
      assert.equal(body.moment, 100); assert.deepEqual(body.extent, [0, 180]);
      assert.equal(body.mix?.tag, 'NEW'); assert.equal(body.mix?.parts?.[0].value, '7');
      assert.equal(state.portals[0].props.label!.name, 'NEW at 100');
      props.widget.time = { mode: 'pin', windows: {} }; render();
      assert.equal(state.portals[0].props.label, null); assert.equal(state.portals[0].props.hour, null);
      assert.equal(state.charts.length, 0, 'a missing saved moment cannot display a guessed branch-end body');
      props.widget.time = { mode: 'pin', windows: { track: { firstOffset: 100, lastOffset: 100 } } }; body = render();
      body.onPick!(20); body = render();
      assert.equal(body.mix?.tag, 'OLD'); assert.equal(body.mix?.parts?.[0].value, '5');
      assert.equal(state.portals[0].props.label!.flag, 'flag:OLD:20');
      body.onPick!(90); body = render();
      assert.equal(body.mix?.parts?.[0].value, 'Not known'); assert.equal(state.portals[0].props.label, null);
      button(body.beforeChart)!(); body = render();
      assert.equal(body.moment, 120); assert.equal(state.portals[0].props.label!.name, 'NEW at 120');
      assert.equal(state.subscriptions.at(-1), true);
      state.hour = 123; body = render(); assert.equal(body.moment, 130);
      const savedTime = JSON.stringify(props.widget.time);
      button(body.beforeChart)!(); body = render(); assert.equal(body.moment, 130);
      state.hour = 133; body = render(); assert.equal(body.moment, 130, 'pause retains the displayed branch hour');
      assert.equal(state.subscriptions.at(-1), false);
      button(body.beforeChart)!(); body = render(); assert.equal(body.moment, 140, 'Follow resumes the independent clock');
      assert.equal(JSON.stringify(props.widget.time), savedTime, 'local pause does not rewrite the saved pin');
      state.hour = 123; body = render();
      state.reply = undefined; body = render(); assert.equal(body.mix?.parts?.[0].value, '7');
      assert.equal(state.portals[0].props.label!.name, 'NEW at 130'); state.reply = reply; render();
      const detailChart = state.charts[0];
      assert.equal(detailChart.tooltip!(20, {} as never, [])?.title, 'OLD at 20');
      detailChart.onContextMenu!(20, 0, 0); state.menu[0].run(); assert.equal(state.sought.at(-1), 20);
      state.lists[0].onPick!('mod type'); body = render(); assert.equal(body.mix?.bands?.length, 1);
      body.onPick!(20);
      const staleFollow = button(render().beforeChart)!;
      props.runtime.client = { ...props.runtime.client }; render(); staleFollow(); body = render();
      assert.equal(body.moment, 100, 'a previous client cannot switch the replacement pin into Follow');
      props.widget.options = { ...props.widget.options, holder: JSON.stringify(['campaign', '1', 'OLD', 7, 999]) };
      body = render(); assert.equal(body.mix?.parts?.[0].value, 'Not known'); assert.equal(state.portals[0].props.label, null);
      props.widget.options = { show: 'breakdown' }; state.slots = [];
      render(); assert.equal(state.portals[0].props.label, null, 'older tag-only pins retain a neutral header');
      props.widget.time = { mode: 'follow' }; props.widget.options = { show: 'breakdown', drillOriginHour: 20 }; state.slots = [];
      render(); assert.equal(state.portals[0].props.label, null, 'older Follow pins without a saved holder retain a neutral header');
      props.widget.options = { show: 'breakdown' }; state.slots = [];
      render(); assert.equal(state.portals[0].props.label, null, 'older Follow pins without an origin retain a neutral header');
      props.widget.time = { mode: 'campaign' }; props.widget.countries = { kind: 'tags', tags: ['NEW'] }; state.slots = [];
      body = render(); assert.equal(state.portals[0].props.label!.name, 'NEW at 180', 'directly opened breakdown resolves its original holder');
      button(body.beforeChart)!(); body = render(); assert.equal(body.moment, 130);
      state.hour = 200; body = render(); assert.equal(body.moment, 207); assert.equal(state.portals[0].props.label, null);

      props.widget.time = { mode: 'follow' }; props.widget.countries = { kind: 'group', group: { type: 'top', count: 1 }, membership: 'hour', aggregate: false };
      state.slots = []; state.hour = 13; body = render(); assert.equal(body.mix?.tag, 'OLD');
      state.hour = 53; body = render(); assert.equal(body.mix?.tag, 'NEW',
        'a directly opened top-country breakdown keeps its original holder when the ranked tag changes');
      assert.equal(state.portals[0].props.label, null, 'holder-less Follow breakdowns share the legacy pin shape');

      props.widget.time = { mode: 'range', firstOffset: 0, lastOffset: 40 };
      props.widget.options = { view: 'lines' }; props.widget.countries = { kind: 'tags', tags: ['OLD'] }; state.slots = []; state.hour = 113;
      render(); state.charts[0].onPick!(20, { frame: plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 40], [0, 120], false), until: 40 }); render();
      assert.deepEqual(state.views[0].extent, [0, 180]); state.host!.onFollow(); render();
      assert.equal(state.views[0].moment, 120); state.host!.onPin!();
      assert.equal(state.pins.at(-1)!.options?.drillOriginHour, 20);
      assert.deepEqual(JSON.parse(state.pins.at(-1)!.options!.holder as string), ['campaign', '1', 'OLD', 7, 101]);
      assert.equal(state.pins.at(-1)!.time.mode, 'follow');
      state.host!.onPause!(); render(); state.hour = 133; render();
      assert.equal(state.views[0].moment, 120);
      state.host!.onPin!();
      const pausedPin = JSON.parse(JSON.stringify(state.pins.at(-1)!)) as WidgetSpec;
      assert.deepEqual(pausedPin.time, { mode: 'pin', windows: { track: { firstOffset: 120, lastOffset: 120 } } });
      assert.equal(pausedPin.options?.drillOriginHour, 20);
      const sourceWidget = props.widget;
      props.widget = pausedPin; state.slots = []; render(); render();
      assert.equal(state.views[0].moment, 120, 'saved paused drills restore their displayed moment');
      assert.equal(state.portals[0].props.label!.name, 'NEW at 120');
      props.widget = sourceWidget; state.slots = []; state.hour = 113; render();
      props.active = false; render(); assert.equal(state.subscriptions.at(-1), false);

      const picks: [number, string | undefined][] = [];
      const raceProps: CountryRaceViewProps = { runtime: props.runtime, track: props.runtime.tracks[0], active: true,
        options: {}, view: 'race', extent: [0, 180], moment: 120, playhead: 120, slots: 2, lines: [],
        ranked: [{ tag: 'NEW', entityKey: '[7,101]', hour: 120, value: 7, label: '7', stretch: 0 },
          { tag: 'OLD', entityKey: '[8,102]', hour: 120, value: 99, label: '99', stretch: 100 }],
        label: 'Native race', empty: 'Empty', formatValue: String,
        countryLookHour: (_tag, hour) => hour,
        entityLooks: { name: (tag, hour) => `${tag} at ${hour}`, color: () => '#777',
          countryFlag: (tag, hour) => ({ flag: `flag:${tag}:${hour}` }) },
        tooltip: (tag, hour, name) => [{ title: name, rows: [{ label: tag, value: String(hour) }] }],
        onPick: (hour, tag) => picks.push([hour, tag]) };
      state.races = [];
      renderToStaticMarkup(createElement(RaceView, raceProps));
      const race = state.races[0], kept = race.items[0];
      assert.equal(kept.key, '[7,101]'); assert.equal(race.name(kept.key, kept), 'NEW at 120');
      assert.equal(race.countryFlag!(kept.key, kept)?.flag, 'flag:NEW:120');
      assert.equal(race.tooltip!(kept.key, kept)?.rows[0].label, 'NEW');
      race.onPick!(kept.key, kept); assert.deepEqual(picks, [[120, 'NEW']]);
      race.onContextMenu!(kept.key, 0, 0, kept); state.menu[0].run(); assert.equal(state.sought.at(-1), 120);
      const departing = { ...kept, tag: 'OLD', hour: 20 };
      assert.equal(race.name(departing.key, departing), 'OLD at 20');
      assert.equal(race.countryFlag!(departing.key, departing)?.flag, 'flag:OLD:20');
      assert.equal(race.tooltip!(departing.key, departing)?.title, 'OLD at 20');
      raceProps.countryLookHour = () => null; state.races = [];
      renderToStaticMarkup(createElement(RaceView, raceProps));
      assert.equal(state.races[0].countryFlag!(kept.key, kept)?.flag, undefined,
        'a missing dated read never falls back to a current country flag');

      props.active = true; props.widget.time = { mode: 'pin', windows: { track: { firstOffset: 100, lastOffset: 100 } } };
      props.widget.countries = { kind: 'tags', tags: ['OLD'] };
      props.widget.options = { show: 'breakdown', drillOriginHour: 20, holder: JSON.stringify(['campaign', '1', 'OLD', 7, 101]) };
      state.slots = []; state.reply = reply; state.hour = 113;
      let countryPartsMoment = 100, countryPartsDefaultTag = 'OLD';
      const renderCountryParts = () => {
        state.cursor = 0; state.effects = []; state.views = []; state.charts = []; state.portals = []; state.demands = [];
        renderToStaticMarkup(createElement(CountryPartsDrill<NativePartsRaceReply>, {
          ...props, track: props.runtime.tracks[0], options: { show: 'breakdown' }, extent: [0, 180], moment: countryPartsMoment, playhead: null,
          product: 'country-parts', params: tag => ({ tags: [tag] }), validate: value => value as NativePartsRaceReply,
          decode: value => partsRaceDrillData(partsRaceModel(value), value.series, () => null), numbers: nativePartsRaceNumbers,
          defaultTag: countryPartsDefaultTag, pickTag: () => 'OLD', title: 'Country parts', measure: 'Total', warnings: [], resetKey: 'country-parts', main: () => null,
        }));
        for (const effect of state.effects) effect();
        return state.views[0];
      };
      body = renderCountryParts(); assert.equal(state.portals[0].props.label!.name, 'NEW at 100');
      props.widget.options = { show: 'breakdown', drillOriginHour: 20 }; renderCountryParts();
      assert.equal(state.portals[0].props.label, null, 'an origin-only legacy pin retains a neutral header');
      assert.equal(state.views[0].mix?.parts?.[0].value, '7', 'its native origin still binds the body');
      props.widget.options = { show: 'breakdown', drillOriginHour: 20, holder: JSON.stringify(['campaign', '1', 'OLD', 7, 101]) };
      props.widget.time = { mode: 'pin', windows: {} }; renderCountryParts();
      assert.equal(state.portals[0].props.label, null); assert.equal(state.portals[0].props.hour, null); assert.equal(state.charts.length, 0);
      props.widget.time = { mode: 'pin', windows: { track: { firstOffset: 100, lastOffset: 100 } } }; body = renderCountryParts();
      body.onPick!(20); body = renderCountryParts(); assert.equal(state.portals[0].props.label!.name, 'OLD at 20');
      button(body.beforeChart)!(); body = renderCountryParts(); assert.equal(state.portals[0].props.label!.flag, 'flag:NEW:120');
      button(body.beforeChart)!(); body = renderCountryParts(); assert.equal(body.moment, 120);
      state.hour = 133; body = renderCountryParts(); assert.equal(body.moment, 120, 'country-parts pause retains owner and date');
      button(body.beforeChart)!(); body = renderCountryParts(); assert.equal(body.moment, 140);
      props.widget.time = { mode: 'follow' }; state.slots = []; countryPartsMoment = 140; body = renderCountryParts(); renderCountryParts();
      button(body.beforeChart)!(); body = renderCountryParts();
      state.hour = 143; countryPartsMoment = 150; body = renderCountryParts();
      assert.equal(body.moment, 140, 'a restored Follow pin can pause before it has any local drill state');
      button(body.beforeChart)!(); body = renderCountryParts(); assert.equal(body.moment, 150);
      props.widget.time = { mode: 'pin', windows: { track: { firstOffset: 100, lastOffset: 100 } } };
      countryPartsMoment = 100; state.hour = 113;
      state.charts[0].onContextMenu!(20, 0, 0); state.menu[0].run(); assert.equal(state.sought.at(-1), 20);
      props.widget.options = { show: 'breakdown', drillOriginHour: 90 }; renderCountryParts();
      assert.equal(state.portals[0].props.label, null); assert.equal(state.charts.length, 0, 'an invalid saved native origin refuses the body');
      props.widget.options = { show: 'breakdown', drillOriginHour: 20, holder: 'invalid saved holder' }; renderCountryParts();
      assert.equal(state.portals[0].props.label, null); assert.equal(state.charts.length, 0);
      props.widget.options = { show: 'breakdown', drillOriginHour: 20, holder: JSON.stringify(['campaign', '1', 'OLD', 7, 999]) }; renderCountryParts();
      assert.equal(state.portals[0].props.label, null); assert.equal(state.charts.length, 0, 'a mismatched saved holder refuses another country');
      props.widget.options = { show: 'breakdown' }; renderCountryParts(); assert.equal(state.portals[0].props.label, null,
        'originless country-parts pins keep a neutral header');
      props.widget.time = { mode: 'campaign' };
      props.widget.countries = { kind: 'group', group: { type: 'top', count: 1 }, membership: 'hour', aggregate: false };
      state.slots = []; countryPartsDefaultTag = 'NEW'; body = renderCountryParts();
      assert.equal(state.portals[0].props.label!.name, 'NEW at 100');
      countryPartsMoment = 110; countryPartsDefaultTag = 'OLD'; body = renderCountryParts();
      assert.equal(body.mix?.parts?.[0].value, '7'); assert.equal(state.portals[0].props.label!.name, 'NEW at 110',
        'a directly opened country-parts breakdown keeps its holder when the top country changes');
    }
  } finally { await vite.close(); delete root.__partsIdentity; delete root.__partsHooks; }
});
