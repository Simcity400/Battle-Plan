import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { CountryRaceViewProps } from './CountryRaceView';
import type { TimeChartProps } from '../TimeChart';
import type { RemoteRaceRows } from './RaceChart';
import type { PlotFrame } from '../frame';
import { plotFrame } from '../frame.ts';
import { tooltipHeight, tooltipRoom, tooltipText } from '../tooltip.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import * as summaryHelpers from '../chartSummary.ts';

test('actual country time views forward dated badges, keyboard stops and full event tooltips without changing picks', async () => {
  type Slot = { value?: unknown; deps?: readonly unknown[] };
  const state = { cursor: 0, slots: [] as Slot[], wanted: [] as { hour: number; tag: string }[], ready: true };
  const hook = (kind: string, ...args: unknown[]) => {
    const slot = state.slots[state.cursor++] ??= {};
    if (kind === 'useEffect') return;
    if (kind === 'useMemo' || kind === 'useCallback') {
      const [factory, deps] = args as [() => unknown, readonly unknown[]];
      if (!slot.deps || slot.deps.length !== deps.length || !slot.deps.every((value, index) => Object.is(value, deps[index]))) {
        slot.deps = [...deps]; slot.value = kind === 'useCallback' ? factory : factory();
      }
      return slot.value;
    }
    if (!('value' in slot)) slot.value = kind === 'useRef' ? { current: args[0] }
      : typeof args[0] === 'function' ? (args[0] as () => unknown)() : args[0];
    if (kind === 'useRef') return slot.value;
    return [slot.value, (next: unknown) => { slot.value = typeof next === 'function' ? (next as (before: unknown) => unknown)(slot.value) : next; }];
  };
  const replacements: Record<string, string> = {
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => {
      const state = globalThis.__countryMarkerFacts; state.wanted = wanted;
      return { labelAt: hour => tag => { const known = state.ready && wanted.some(read => read.hour === hour && read.tag === tag);
        return known ? { name: tag + ' at ' + hour, flagReason: 'Exact dated flag refused', flagPending: false }
          : { name: tag, flagPending: true }; }, revision: state.ready ? 1 : 0 }; };`,
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => globalThis.__countryMarkerHeight ?? 1000; export const useChartTokens = () => ({ categories: ["green"], muted: "gray", fontSize: { body: 14 } });',
    '/charts/chartSummary.ts': 'export const drawApproximateSpans = (...args) => globalThis.__countrySummaryHelpers.drawApproximateSpans(...args); export const extentOfSummary = (...args) => globalThis.__countrySummaryHelpers.extentOfSummary(...args); export const pickSummaryLines = (...args) => globalThis.__countrySummaryHelpers.pickSummaryLines(...args); export const drawLineSummary = (_ctx, _frame, _tokens, summary) => globalThis.__countrySummaries.push(summary);',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "blue";',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__countryMarkerTime = props; return null; };',
    '/charts/race/RaceChart.tsx': 'export default props => { globalThis.__countryMarkerRace = props; return null; };',
  };
  const host = globalThis as typeof globalThis & { __countryMarkerTime?: TimeChartProps; __countryMarkerRace?: { badges?: unknown; nameSamples?: readonly string[]; countryFlag?: (tag: string) => { flagDetail?: string }; remoteRows?: RemoteRaceRows }; __countryMarkerHeight?: number;
    __countrySummaries?: unknown[]; __countrySummaryHelpers?: typeof summaryHelpers; __countryMarkerHook?: typeof hook; __countryMarkerFacts?: typeof state };
  host.__countryMarkerHook = hook; host.__countryMarkerFacts = state;
  host.__countrySummaryHelpers = summaryHelpers;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'country-marker-forwarding', enforce: 'pre',
      transform: (source, id) => {
        const key = Object.keys(replacements).find(key => id.endsWith(key));
        if (key) return replacements[key];
        // The looks' reads are synthetic; which hour and hours the view asks them at is the module's own.
        if (id.endsWith('/dashboard/countryLooks.ts')) return source.replace('export function useCountryLooks(', 'function readCountryLooks(')
          + '\nexport const useCountryLooks = () => ({ name: tag => tag, flag() {}, flagDetail: () => "Synthetic sampled flag decoder failure", flagReason: () => "The flag request failed.", color() {}, revision: 0 });';
        if (id.endsWith('/charts/race/CountryRaceView.tsx')) return source.replace(/\b(useMemo|useCallback|useState|useRef|useEffect)(?:<[\s\S]*?>)?\(/g,
          (_match, kind) => `globalThis.__countryMarkerHook(${JSON.stringify(kind)}, `);
      },
    }] });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/charts/race/CountryRaceView.tsx') as typeof import('./CountryRaceView');
    const badges = [{ hour: 15, glyph: '\u00d7', label: 'native alias capitulated', capitulationTag: 'native alias' }];
    const run = { firstHour: 10, lastHour: 20, value: 3 };
    const pickOwner = {};
    const props = { runtime: { client: {}, clock: { getHour: () => 16 } }, track: { id: 'track', source: { id: 'source' }, branch: { id: 'branch', firstHour: 10, lastHour: 20 } },
      active: true, options: { playback: 'cursor' }, view: 'lines', extent: [10, 20], playhead: 16, moment: 16, slots: 1,
      ranked: [{ tag: 'native alias', value: 3 }], lines: [{ tag: 'native alias', runs: [run] }], keyHours: [10, 20], badges, pickOwner,
      label: 'Army', empty: 'No army reads', formatValue: String, tooltip: () => null,
      lineTooltip: () => [{ title: 'Exact numeric read', rows: [{ label: 'Native count', value: '3' }], spoken: 'Exact native count 3' }],
    } as unknown as CountryRaceViewProps;
    const render = () => { state.cursor = 0; return renderToStaticMarkup(createElement(View, props)); };
    assert.match(render(), /data-detail="Synthetic sampled flag decoder failure"/);
    props.view = 'race'; render();
    assert.equal(host.__countryMarkerRace!.countryFlag!('native alias').flagDetail, 'Synthetic sampled flag decoder failure');
    props.view = 'lines'; render();
    assert.deepEqual(state.wanted, [], 'drawn markers do not preload any dated country names or flags');
    let chart = host.__countryMarkerTime!;
    assert.equal(chart.badges, badges);
    assert.deepEqual(chart.keyHours, [10, 15, 20]);
    assert.equal(chart.pickOwner, pickOwner);
    assert.equal(props.lines[0].runs[0], run);
    chart.tooltip!(15, {} as PlotFrame, badges); render(); chart = host.__countryMarkerTime!;
    assert.deepEqual([...new Set(state.wanted.map(read => JSON.stringify(read)))], [JSON.stringify({ hour: 15, tag: 'native alias' })]);
    const tip = chart.tooltip!(15, {} as PlotFrame, badges)!;
    assert.equal(tip.rows.find(row => row.label === 'Native count')?.value, '3');
    assert.equal(tip.rows[0].label, '\u00d7 native alias at 15 capitulated');
    assert.equal(tip.rows[0].countryFlag?.flagReason, 'Exact dated flag refused');
    assert.match(tooltipText(tip), /Exact native count 3.*native alias at 15 capitulated/);

    props.lineTooltip = () => [];
    render();
    const emptyLines = host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, badges)!;
    assert.equal(emptyLines.rows[0].label, '\u00d7 native alias at 15 capitulated');
    assert.equal(emptyLines.title, formatDateTime(15));
    assert.deepEqual(host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, []), { title: '', rows: [] }, 'without events the prior empty fitter behavior is retained');

    props.view = 'mix'; props.mix = { tag: 'native alias', bands: [[run]] };
    render();
    chart = host.__countryMarkerTime!;
    assert.equal(chart.badges, badges);
    assert.equal(chart.pickOwner, pickOwner);
    assert.equal(chart.tooltip!(15, {} as PlotFrame, badges)?.rows[0].label, '\u00d7 native alias at 15 capitulated', 'a known event survives missing numeric reads');
    props.timeTooltip = () => null;
    render();
    assert.equal(host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, badges)?.rows[0].label, '\u00d7 native alias at 15 capitulated');
    props.presentation = { countryLooks: false }; props.countryLookHour = () => 12;
    render(); assert.equal(host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, []), null); render();
    assert.deepEqual(state.wanted, [], 'custom tooltips keep ownership of their numeric names and flags');
    host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, badges); render();
    assert.deepEqual(state.wanted, [{ hour: 15, tag: 'native alias' }], 'custom tooltips add only reached event-date requests');
    assert.equal(host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, badges)?.rows[0].label, '\u00d7 native alias at 15 capitulated');
    props.presentation = {}; props.countryLookHour = undefined;

    let iterations = 0;
    const empty = (function* () { iterations++; })();
    props.timeTooltip = () => empty;
    render();
    const emptyTime = host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, badges)!;
    assert.equal(emptyTime.rows[0].label, '\u00d7 native alias at 15 capitulated');
    assert.equal(emptyTime.title, formatDateTime(15));
    assert.equal(iterations, 1, 'single-pass empty forms are consumed only once');

    props.countryLookHour = () => null; props.lines = []; props.view = 'lines'; props.timeTooltip = undefined;
    render(); host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, badges); render();
    assert.deepEqual(state.wanted, [{ hour: 15, tag: 'native alias' }], 'the event keeps its own name read even when no numeric supplying read exists');
    state.ready = false; render();
    const pending = host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, badges)!;
    assert.equal(pending.rows[0].label, '\u00d7 native alias capitulated'); assert.equal(pending.rows[0].countryFlag?.flagPending, true);
    assert.doesNotMatch(tooltipText(pending), /Flag loading|Exact dated flag refused/);
    state.ready = true; props.countryLookHour = () => 12;
    const simultaneous = Array.from({ length: 5000 }, (_, index) => ({ hour: 15, glyph: '\u00d7', label: `opaque actor ${index} capitulated`, capitulationTag: `opaque actor ${index}` }));
    host.__countryMarkerHeight = 250; props.view = 'lines'; props.timeTooltip = undefined;
    props.badges = simultaneous; props.lines = simultaneous.map((_, index) => ({ tag: `opaque actor ${index}`, runs: [run] }));
    props.lineTooltip = () => [{ title: 'Exact native total', rows: [{ label: 'Total', value: '15000' }] }];
    render(); host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, simultaneous); render();
    const crowded = host.__countryMarkerTime!.tooltip!(15, {} as PlotFrame, simultaneous)!;
    assert.equal(host.__countryMarkerTime!.badges?.length, 5000);
    assert.ok(crowded.rows.some(row => (row.omittedRows ?? 0) > 0), 'unshown native rows retain an explicit count');
    assert.ok(tooltipHeight(crowded) <= tooltipRoom(host.__countryMarkerHeight), 'simultaneous events fit the measured viewport');
    assert.match(tooltipText(crowded), /opaque actor 4999 at 15 capitulated/);
    assert.match(tooltipText(crowded), /Total 15000/);
    assert.equal(state.wanted.filter((read: { hour: number; tag: string }) => read.hour === 12).length, 5000, 'numeric rows retain their supplying read');
    assert.equal(state.wanted.filter((read: { hour: number; tag: string }) => read.hour === 15).length, 5000, 'all reached events retain their actual date');
    host.__countryMarkerTime!.onHover!(null); render();
    assert.deepEqual(state.wanted, [], 'clearing the actual TimeChart hover releases reached names and flags');

    const resistance = { from: 10, to: 20, width: 1, buckets: [null], runs: undefined };
    const compliance = { ...resistance, buckets: [null, null] };
    props.view = 'lines'; props.lines = ['resistance', 'compliance'].map(measure => ({ tag: 'native alias', id: JSON.stringify(['native alias', measure]), runs: [run] }));
    props.lineSummaries = new Map([[props.lines[0].id!, resistance], [props.lines[1].id!, compliance]]);
    host.__countrySummaries = [];
    render();
    for (const layer of host.__countryMarkerTime!.layers)
      layer.draw({} as never, { x: [10, 20] } as never, {} as never, {} as never);
    assert.deepEqual(host.__countrySummaries, [resistance, compliance], 'two measures of one country draw their distinct prepared summaries');
    props.lineSummaries = undefined;

    delete host.__countryMarkerTime; props.view = 'race';
    render();
    assert.equal(host.__countryMarkerTime, undefined);
    assert.equal(host.__countryMarkerRace?.badges, undefined, 'race bars do not acquire a marker strip');
    props.presentation = {}; render();
    assert.deepEqual(host.__countryMarkerRace?.nameSamples, ['Country name unavailable'], 'native-country names reserve the same room before or after their reads');
    props.entityLooks = { name: () => 'Authored aggregate', color: () => 'blue' }; render();
    props.ranked = []; props.remoteRows = { total: 5000, offset: 0, min: -10, max: 100, owner: {}, onRange() {} };
    render();
    assert.equal(host.__countryMarkerRace?.remoteRows, props.remoteRows, 'pending real rows forward the global contract rather than becoming an empty race');
    assert.equal(host.__countryMarkerRace?.nameSamples, undefined, 'custom aggregate names retain their own sizing');

    const frame = plotFrame(400, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [10, 20], [0, 10], false);
    const summary = (value: number) => ({ from: 10, to: 20, width: Math.ceil(frame.plotWidth), buckets: [],
      runs: [{ firstHour: 10, lastHour: 20, value, breakBefore: true }] });
    props.view = 'lines'; props.playhead = null; props.badges = []; props.entityLooks = undefined;
    props.lines = [{ tag: 'native alias', id: 'fuel', runs: [] }, { tag: 'native alias', id: 'capacity', runs: [] },
      { tag: 'nearby actor', runs: [] }];
    props.lineSummaries = new Map([['fuel', summary(3)], ['capacity', summary(8)], ['nearby actor', summary(7)]]);
    const picks: { hour: number; tag?: string; choices?: readonly string[] }[] = [];
    props.exactLineTags = async () => ['native alias', 'nearby actor'];
    props.onPick = (hour, tag, choices) => picks.push({ hour, tag, choices });
    render(); chart = host.__countryMarkerTime!;
    assert.deepEqual(chart.layers.find(layer => layer.extent)!.extent!([10, 20]), [3, 8], 'fuel and capacity use their separate prepared summary extents');
    const context = { frame, until: null, point: { x: frame.toX(15), y: frame.toY(8) } };
    await chart.onPick!(15, context);
    assert.deepEqual(picks, [{ hour: 15, tag: 'native alias', choices: ['native alias'] }],
      'a capacity pick resolves its country without a second measure overwriting the closest geometry');
    let resolveKnown!: (tags: readonly string[]) => void;
    props.exactLineTags = () => new Promise(resolve => { resolveKnown = resolve; });
    render(); chart = host.__countryMarkerTime!;
    const pendingPick = chart.onPick!(15, context);
    props.pickOwner = {}; render(); resolveKnown(['native alias', 'nearby actor']);
    await pendingPick;
    assert.equal(picks.length, 1, 'a changed numeric owner revokes a pending prepared pick');
    props.lineSummaries=new Map();props.exactLineTags=undefined;props.lines=[{tag:'native alias',runs:[run]}];
    render();await host.__countryMarkerTime!.onPick!(15,{frame,until:null,point:{x:frame.toX(15),y:frame.toY(3)}});
    assert.equal(picks.length,2,'an empty prepared map retains ordinary drawn-line picks');
    assert.equal(picks[1].tag,'native alias');
    props.view='mix';props.lineSummaries=undefined;props.mix={tag:'native alias',bands:[[run]],parts:[{id:'test',label:'Test'}],approximate:[{firstHour:12,lastHour:15}]};
    render();
    const alpha:number[]=[];
    const canvas=new Proxy({globalAlpha:1,fill(){alpha.push(this.globalAlpha);}}, {get(target,key){return key in target ? target[key as keyof typeof target] : ()=>{};}}) as unknown as CanvasRenderingContext2D;
    for (const layer of host.__countryMarkerTime!.layers)
      layer.draw(canvas,frame,{} as never,{playhead:null} as never);
    assert.ok(alpha.includes(0.55),'mix bands use the real shared approximate clipping and faded drawing');
    assert.ok(alpha.includes(1),'settled bands retain the original opacity');
  } finally {
    delete host.__countryMarkerTime; delete host.__countryMarkerRace; delete host.__countryMarkerHeight;
    delete host.__countrySummaries;
    delete host.__countrySummaryHelpers;
    delete host.__countryMarkerHook; delete host.__countryMarkerFacts;
    await vite.close();
  }
});
