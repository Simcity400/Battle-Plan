import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { CountryRaceViewProps } from './CountryRaceView';
import type { TimeChartProps } from '../TimeChart';
import type { PlotFrame } from '../frame';
import { tooltipHeight, tooltipRoom, tooltipText } from '../tooltip.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';

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
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag() {}, color() {}, revision: 0 });',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => {
      const state = globalThis.__countryMarkerFacts; state.wanted = wanted;
      return { labelAt: hour => tag => { const known = state.ready && wanted.some(read => read.hour === hour && read.tag === tag);
        return known ? { name: tag + ' at ' + hour, flagReason: 'Exact dated flag refused', flagPending: false }
          : { name: tag, flagPending: true }; }, revision: state.ready ? 1 : 0 }; };`,
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => globalThis.__countryMarkerHeight ?? 1000; export const useChartTokens = () => ({ categories: ["green"], muted: "gray", fontSize: { body: 14 } });',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "blue";',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__countryMarkerTime = props; return null; };',
    '/charts/race/RaceChart.tsx': 'export default props => { globalThis.__countryMarkerRace = props; return null; };',
  };
  const host = globalThis as typeof globalThis & { __countryMarkerTime?: TimeChartProps; __countryMarkerRace?: { badges?: unknown }; __countryMarkerHeight?: number;
    __countryMarkerHook?: typeof hook; __countryMarkerFacts?: typeof state };
  host.__countryMarkerHook = hook; host.__countryMarkerFacts = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'country-marker-forwarding', enforce: 'pre',
      transform: (source, id) => {
        const key = Object.keys(replacements).find(key => id.endsWith(key));
        if (key) return replacements[key];
        if (id.endsWith('/charts/race/CountryRaceView.tsx')) return source.replace(/\b(useMemo|useCallback|useState|useRef|useEffect)(?:<[\s\S]*?>)?\(/g,
          (_match, kind) => `globalThis.__countryMarkerHook(${JSON.stringify(kind)}, `);
      },
    }] });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/charts/race/CountryRaceView.tsx') as typeof import('./CountryRaceView');
    const badges = [{ hour: 15, glyph: '\u00d7', label: 'native alias capitulated', capitulationTag: 'native alias' }];
    const run = { firstHour: 10, lastHour: 20, value: 3 };
    const pickOwner = {};
    const props = { runtime: { client: {}, clock: {} }, track: { id: 'track', source: { id: 'source' }, branch: { id: 'branch', firstHour: 10, lastHour: 20 } },
      active: true, options: { playback: 'cursor' }, view: 'lines', extent: [10, 20], playhead: 16, moment: 16, slots: 1,
      ranked: [{ tag: 'native alias', value: 3 }], lines: [{ tag: 'native alias', runs: [run] }], keyHours: [10, 20], badges, pickOwner,
      label: 'Army', empty: 'No army reads', formatValue: String, tooltip: () => null,
      lineTooltip: () => [{ title: 'Exact numeric read', rows: [{ label: 'Native count', value: '3' }], spoken: 'Exact native count 3' }],
    } as unknown as CountryRaceViewProps;
    const render = () => { state.cursor = 0; renderToStaticMarkup(createElement(View, props)); };
    render();
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

    delete host.__countryMarkerTime; props.view = 'race';
    render();
    assert.equal(host.__countryMarkerTime, undefined);
    assert.equal(host.__countryMarkerRace?.badges, undefined, 'race bars do not acquire a marker strip');
  } finally {
    delete host.__countryMarkerTime; delete host.__countryMarkerRace; delete host.__countryMarkerHeight;
    delete host.__countryMarkerHook; delete host.__countryMarkerFacts;
    await vite.close();
  }
});
