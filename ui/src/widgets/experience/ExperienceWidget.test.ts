import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply, ExperienceRead, ExperienceReply } from '../../generated/dashboard';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { PlotFrame } from '../../charts/frame';
import { plotFrame } from '../../charts/frame.ts';
import type { TooltipRow } from '../../charts/tooltip';
import type { ChartTokens } from '../../charts/tokens';
import type { EventBadge } from '../../charts/badges';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';

test('actual Experience keeps holder aliases, dated reached events and one end per branch', async () => {
  const read: ExperienceRead = { firstHour: 0, lastHour: 11, readHour: 0, startsSegment: true,
    armyRaw: '32768', navyRaw: '0', airRaw: '0', change: null, changeUnknown: 'noMatchingRead', gain: null, gainUnknown: 'noCompleteDay' };
  const data: ExperienceReply = { branchId: '1', scale: 32768, dayHours: 24, warnings: [], series: [
    { tag: 'A', identities: [{ firstHour: 0, lastHour: 11, slot: 1, countryId: 7 }], reads: [read] },
    { tag: 'B', identities: [{ firstHour: 12, lastHour: 23, slot: 1, countryId: 7 }], reads: [{ ...read, firstHour: 12, lastHour: 23 }] },
  ] };
  const groups: CountryGroupReply = { branchId: '1', humans: [{ tag: 'A', firstHour: 0, lastHour: 11 }], factions: [], wars: [], warStarts: [],
    countryIdentities: data.series.map(({ tag, identities }) => ({ tag, identities: identities! })),
    capitulations: [{ tag: 'B', hour: 15 }], identityChanges: [], unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
  const state = { data, groups, tip: { hour: 15, events: [] } as { hour: number; events: readonly EventBadge[] } | null,
    labels: new Map<string, ReturnType<CountryLabel>>(), chart: undefined as TimeChartProps | undefined,
    looks: [] as readonly string[], wanted: [] as { tag: string; hour: number }[], names: [] as { tag: string; hour: number }[], ends: [] as { id: string; text: string }[],
    draws: [] as { firstHour: number | undefined; endDot: boolean | undefined; until: number | undefined; visible: boolean }[] };
  const host = globalThis as typeof globalThis & { __experienceIdentity?: typeof state };
  host.__experienceIdentity = state;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__experienceIdentity.data, retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__experienceIdentity.groups });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 23; export const usePlaybackSpeed = () => 0;',
    '/dashboard/countryLooks.ts': `export const lookPlan = (_branch, tags, hour) => ({wanted: tags.map(tag => ({tag, hour})), maxHours: 16});
      export const useCountryLooks = (_client, _track, tags) => { globalThis.__experienceIdentity.looks = tags; return { color: () => [20, 50, 80], warnings: [] }; };`,
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesRead = (_client, _track, wanted) =>
      new Map(wanted.map(row => [nameKey(row.hour, row.tag), row.tag]));`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => { globalThis.__experienceIdentity.wanted = wanted;
      return { revision: 0, labelAt: hour => tag => { const state = globalThis.__experienceIdentity; state.names.push({tag, hour});
        return state.labels.get(JSON.stringify([hour, tag])) ?? {name: tag, known: true}; } }; };`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456', '#654321', '#abcdef'], surface: '#000000', countryColours: 'game' });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__experienceIdentity.chart = props; return null; };',
    '/charts/draw.ts': `import { lineEndHour } from './series.ts'; export const drawLine = (_ctx, frame, _tokens, runs, _color, options) => {
      globalThis.__experienceIdentity.draws.push({firstHour: runs[0]?.firstHour, endDot: options.endDot, until: options.until,
        visible: lineEndHour(runs, frame.x[0], Math.min(frame.x[1], options.until ?? Infinity)) !== null}); };
      export const placeEndLabels = (_ctx, _frame, _tokens, ends) => { globalThis.__experienceIdentity.ends = ends; };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'experience-identity-wiring', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/widgets/experience/ExperienceWidget.tsx')) {
          const tipState = 'useState<{ owner: object; hour: number; events: readonly EventBadge[] } | null>(null)';
          assert.ok(source.includes(tipState));
          // Seed each SSR render and observe the actual chart callbacks' next demand.
          return source.replace(tipState, `[globalThis.__experienceIdentity.tip === null ? null : {...globalThis.__experienceIdentity.tip, owner: tipOwner},
            value => { globalThis.__experienceIdentity.tip = value; }] as const`);
        }
      },
    }] });
  try {
    const { default: registered } = await vite.ssrLoadModule('/ui/src/widgets/experience/index.ts') as typeof import('./index.ts');
    const track = { id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } },
      branch: { id: '1', firstHour: 0, lastHour: 23 } };
    const props = { widget: { id: 'experience', kind: 'experience', trackIds: ['main'], countries: { kind: 'tags', tags: ['A'] },
      time: { mode: 'campaign' }, options: {} }, runtime: { tracks: [track], clock: { getHour: () => 23 }, client: {} },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    for (const membership of ['hour', 'ever'] as const) {
      props.widget.countries = { kind: 'group', group: { type: 'humans' }, membership, aggregate: false };
      renderToStaticMarkup(createElement(registered.render, props));
      assert.deepEqual(state.looks, ['A']);
      assert.equal(state.chart!.layers?.length, 3);
      assert.equal(state.chart!.layers[0].extent?.([12, 23]), null);
      assert.deepEqual(state.chart!.badges?.filter(badge => badge.label === 'B capitulated'), []);
      assert.ok(state.wanted.every(row => row.tag !== 'B'));
    }
    props.widget.countries = { kind: 'tags', tags: ['A'] };
    renderToStaticMarkup(createElement(registered.render, props));
    assert.deepEqual(state.looks, ['A', 'B']);
    assert.deepEqual(state.wanted, [{ tag: 'B', hour: 15 }]);
    state.names = [];
    const tip = state.chart!.tooltip!(15, {} as PlotFrame, [])!;
    assert.deepEqual(tip.rows.map(row => row.label), ['Army', 'Navy', 'Air']);
    assert.deepEqual(state.names, [{ tag: 'B', hour: 15 }]);
    assert.equal(state.chart!.badges?.filter(badge => badge.capitulationTag === 'B').length, 1);
    const capitulation = state.chart!.badges!.find(badge => badge.capitulationTag === 'B')!;
    const generic = { hour: 15, glyph: '+', label: 'joined' };
    state.tip = { hour: 17, events: [capitulation, generic] };
    state.labels.set(JSON.stringify([17, 'B']), { name: 'B at the hover', known: true, flag: 'hover-flag' });
    const eventLabel = { name: 'B at the event', known: true, flag: 'event-flag', flagPending: false };
    state.labels.set(JSON.stringify([15, 'B']), eventLabel);
    renderToStaticMarkup(createElement(registered.render, props));
    assert.deepEqual(state.wanted, [{ tag: 'B', hour: 17 }, { tag: 'B', hour: 15 }]);
    state.names = [];
    const eventTip = state.chart!.tooltip!(17, {} as PlotFrame, state.tip.events)!;
    assert.deepEqual(state.names, [{ tag: 'B', hour: 15 }, { tag: 'B', hour: 17 }]);
    assert.deepEqual(eventTip.rows.slice(-2).map(row => [row.label, row.countryFlag]),
      [['\u00d7 B at the event capitulated', eventLabel], ['+ joined', undefined]]);
    for (const label of [
      { name: '', known: true, flagPending: true },
      { name: 'B', known: false, nameFailed: true, flagPending: false, flagReason: 'The event flag request failed.' },
    ]) {
      state.labels.set(JSON.stringify([15, 'B']), label);
      const dated: TooltipRow = state.chart!.tooltip!(17, {} as PlotFrame, state.tip!.events)!.rows.at(-2)!;
      assert.equal(dated.label, `\u00d7 ${label.name} capitulated`);
      assert.deepEqual(dated.countryFlag, label);
    }
    state.chart!.onHover!(null);
    assert.equal(state.tip, null);
    renderToStaticMarkup(createElement(registered.render, props));
    assert.deepEqual(state.wanted, []);
    state.chart!.tooltip!(17, {} as PlotFrame, [capitulation]);
    renderToStaticMarkup(createElement(registered.render, props));
    assert.deepEqual(state.wanted, [{ tag: 'B', hour: 17 }, { tag: 'B', hour: 15 }]);
    assert.equal(state.chart!.tooltip!(-1, {} as PlotFrame, [capitulation]), null);
    renderToStaticMarkup(createElement(registered.render, props));
    assert.deepEqual(state.wanted, []);
    state.chart!.overlay!({} as CanvasRenderingContext2D,
      { ...plotFrame(400, 300, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 23], [0, 1], false), toY: (value: number) => value }, {} as ChartTokens);
    assert.equal(state.ends.length, 3);
    assert.deepEqual(state.ends.map(end => end.text.split(' ')[0]), ['Army', 'Navy', 'Air']);
    const draw = (x: [number, number], playhead: number | null) => {
      state.draws = [];
      state.chart!.layers[0].draw({} as CanvasRenderingContext2D, plotFrame(400, 300, { left: 0, right: 0, top: 0, bottom: 0 }, x, [0, 1], false), {} as ChartTokens,
        { playhead });
      assert.ok(state.draws.every(call => call.until === (playhead ?? undefined)));
      return state.draws.filter(call => call.visible && call.endDot !== false).map(call => call.firstHour);
    };
    assert.deepEqual(draw([0, 23], 10), [0]);
    assert.deepEqual(draw([0, 23], 15), [12]);
    assert.deepEqual(state.draws.map(call => call.endDot), [false, true]);
    assert.deepEqual(draw([12, 23], 15), [12]);
    assert.deepEqual(draw([12, 23], 10), []);
    assert.deepEqual(draw([0, 11], null), [0]);
    assert.deepEqual(draw([0, 23], null), [12]);
    assert.deepEqual(draw([24, 30], null), []);
    // The held native holder may resume after a true recording gap.
    state.data = { ...data, series: [data.series[0], { ...data.series[1],
      identities: [{ firstHour: 16, lastHour: 23, slot: 1, countryId: 7 }],
      reads: [{ ...data.series[1].reads[0], firstHour: 16, readHour: 16 }] }] };
    renderToStaticMarkup(createElement(registered.render, props));
    assert.deepEqual(draw([0, 23], 14), [0]);
    assert.deepEqual(draw([12, 15], 14), []);
  } finally {
    delete host.__experienceIdentity;
    await vite.close();
  }
});
