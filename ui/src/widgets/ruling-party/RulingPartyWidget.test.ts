import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import { plotFrame } from '../../charts/frame.ts';
import type { LedgerGovernment, PartyHistoryReply, PoliticsLedgerReply } from '../../generated/dashboard';
import { wireSummary } from '../../charts/chartSummary.fixture.ts';

test('dense ruling-party combines census, native and tail marks and chooses the nearer keyboard event', async t => {
  type Slot = { value?: unknown; deps?: readonly unknown[]; cleanup?: () => void };
  const fixture = { cursor: 0, slots: [] as Slot[], effects: [] as (() => void)[], chart: undefined as TimeChartProps | undefined,
    requests: [] as { product: string; params: Record<string, unknown>; signal: AbortSignal }[] };
  const host = globalThis as typeof globalThis & {
    __politicsHook?: (name: string, ...args: unknown[]) => unknown; __politicsChart?: (props: TimeChartProps) => void;
  };
  host.__politicsChart = props => { fixture.chart = props; };
  // Drive the real widget, product cache and both prepared hooks through commits.
  // Only React scheduling and unrelated chart/clock/colour inputs are replaced.
  host.__politicsHook = (name, ...args) => {
    const slot = fixture.slots[fixture.cursor++] ??= {};
    if (name === 'useMemo' || name === 'useCallback' || name === 'useEffect') {
      const [factory, deps] = args as [() => unknown, readonly unknown[]];
      if (!slot.deps || slot.deps.length !== deps.length || deps.some((value, index) => !Object.is(value, slot.deps![index]))) {
        slot.deps = [...deps];
        if (name === 'useMemo') slot.value = factory();
        else if (name === 'useCallback') slot.value = factory;
        else fixture.effects.push(() => { slot.cleanup?.(); const cleanup = factory(); slot.cleanup = typeof cleanup === 'function' ? cleanup as () => void : undefined; });
      }
      return slot.value;
    }
    if (name === 'useRef') { if (!('value' in slot)) slot.value = { current: args[0] }; return slot.value; }
    if (!('value' in slot)) slot.value = typeof args[0] === 'function' ? (args[0] as () => unknown)() : args[0];
    return [slot.value, (next: unknown) => { slot.value = typeof next === 'function' ? (next as (old: unknown) => unknown)(slot.value) : next; }];
  };
  const replacements: Record<string, string> = {
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__politicsChart(props); return null; };',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/tokens.ts': 'export const readTokens = () => ({ surface: "#101820" }); export const subscribeTokens = () => () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'combined-politics-inputs', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
      if (replacement) return replacement[1];
      if (path.endsWith('/dashboard/contentRevision.ts')) return source.replace('return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);', 'return revision.snapshot();');
      if (['/ruling-party/RulingPartyWidget.tsx', '/ruling-party/preparedParty.ts', '/ruling-party/preparedGovernments.ts', '/dashboard/products.ts', '/dashboard/viewRead.ts'].some(suffix => path.endsWith(suffix)))
        return source.replace(/\b(useMemo|useCallback|useState|useRef|useEffect)(?:<[\s\S]*?>)?\(/g, (_match, name) => `globalThis.__politicsHook(${JSON.stringify(name)}, `);
    } }] });
  const reply = (): PartyHistoryReply => ({ branchId: '1', tag: 'mod country', decimals: 5, warnings: [], ideologies: [], ideologyTypes: [], people: [], leaders: [],
    partyNames: [], partyNameRuns: [], elections: [], unobserved: [], ruling: [{ token: 'mod_a', firstHour: 0, lastHour: 1000, startsSegment: true }],
    parties: ['mod_a', 'mod_b'].map((token, index) => ({ token, runs: [{ firstHour: 0, lastHour: 1000, valueRaw: index ? '3500000' : '4000000', startsSegment: true }] })) });
  const change: LedgerGovernment = { country: 1, instance: 1, epoch: '1', sequence: '1', hour: 60, readHour: 64,
    beforeParty: 1, afterParty: 2, beforeIdeology: 1, afterIdeology: 2, beforeToken: 'mod_a', afterToken: 'mod_b',
    actorCountry: null, originKind: 'OnAction', originToken: null, originKey: [], cause: 'CoupCompletion', covered: true };
  const tail = { ...change, sequence: '3', hour: 90, readHour: 96, cause: 'CivilWar' };
  let partyNext = 107, governmentNext = 102;
  let deferred: ((value: { hour: number }) => void) | undefined;
  let deferGovernment = true;
  const client = {
    product: () => { throw new Error('Both histories must use preparation.'); },
    preparedQuery: async (_source: unknown, product: string, params: Record<string, unknown>, { signal }: { signal: AbortSignal }) => {
      fixture.requests.push({ product, params, signal });
      assert.ok(fixture.requests.length <= 40, 'viewport, reached pages and keyboard requests must remain bounded');
      if (product === 'politics-ledger') {
        assert.equal(params.view, 'governments');
        assert.ok(params.marks || params.navigation || params.page || typeof params.from === 'number', 'no full-branch legacy government request');
        if (params.navigation) return deferGovernment ? new Promise<{ hour: number }>(resolve => { deferred = resolve; }) : { hour: governmentNext };
        if (params.page) return { firstOrdinal: '0', total: '2', cursor: null, warnings: [],
          rows: [change, { ...change, sequence: '2', hour: 65, readHour: 68 }].map((row, index) => ({ tag: params.tag, hour: row.hour, ordinal: String(index), row })) };
        const marks = params.marks as { from: number; to: number; width: number } | undefined;
        const ledger: PoliticsLedgerReply = { branchId: '1', tag: params.tag as string, decimals: 5, warnings: [], spending: [], spendItems: [], spendCategories: [],
          spendingCoverage: [], ideaTooltips: null, commandHolds: [], governments: marks ? [tail] : [change] };
        return marks ? { ...ledger, governmentMarks: { ...marks, clusters: [{ firstHour: 60, lastHour: 65, firstOrdinal: '0', count: '2' }] } } : ledger;
      }
      assert.equal(product, 'party-history');
      if (params.mode === 'next') return { hour: partyNext };
      const data = reply(); data.tag = params.tag as string;
      if (params.mode === 'events') data.elections = [40, 80];
      const summary = params.mode === 'summary';
      const exact = summary && Math.trunc((Number(params.to) - Number(params.from) + 1) / Number(params.width)) < 2;
      if (exact) {
        data.elections = [80];
        data.ruling = [{ token: 'mod_a', firstHour: 0, lastHour: 63, startsSegment: true },
          { token: 'mod_b', firstHour: 64, lastHour: 1000, startsSegment: false }];
      }
      const from = Number(params.from), to = Number(params.to), width = Number(params.width);
      return { coverage: { firstHour: 0, lastHour: 1000, complete: true }, reply: data, cursor: null, total: '2', marks: summary ? ['ruling', 'election'].map(field => ({ field, exact,
        summary: { from: params.from, to: params.to, width: params.width, clusters: [{ firstHour: 40, lastHour: 80, firstOrdinal: '0', count: '2' }] } })) : [],
        summaries: summary ? data.parties.map((party, index) => {
          const value = Number(party.runs[0].valueRaw) / 100000;
          return { token: party.token, summary: wireSummary({ from, to, width,
            approximate: index ? [{ firstHour: 63, lastHour: 65 }, { firstHour: 450, lastHour: 550 }]
              : [{ firstHour: 60, lastHour: 63 }, { firstHour: 400, lastHour: 499 }],
            ...(to - from + 1 <= width ? { runs: party.runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, value, breakBefore: run.startsSegment })) } : {}),
            buckets: [{ firstHour: from, lastHour: to, firstValue: value, lastValue: value, lastValueHour: to,
              minHour: from, maxHour: to, minValue: value, maxValue: value, firstBreak: true, hasBreak: false }] }) };
        }) : [] };
    },
  };
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/ruling-party/RulingPartyWidget.tsx');
    const props = { widget: { kind: 'ruling-party', trackIds: ['track'], countries: { kind: 'tags', tags: ['mod country'] },
      time: { mode: 'range', firstOffset: 0, lastOffset: 1000 }, options: { changes: true } },
      runtime: { client, tracks: [{ id: 'track', axisOffset: 0, source: { id: 'source', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 1000 } }],
        clock: { getHour: () => 1000, getSnapshot: () => ({ hoursPerSecond: 1 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { fixture.cursor = 0; return renderToStaticMarkup(createElement(Widget, props)); };
    const commit = async () => { fixture.effects.splice(0).forEach(run => run()); await new Promise(resolve => setImmediate(resolve)); };
    const settle = async () => {
      let markup = render();
      for (let cycle = 0; cycle < 8; cycle++) {
        await commit(); markup = render();
        if (!fixture.effects.length) return markup;
      }
      assert.fail('The widget did not settle after eight commits.');
    };
    const approximatePaint = (from: number, to: number, width: number) => {
      const frame = plotFrame(width, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [from, to], [0, 100], false);
      const styles: { alpha: number; dash: number[] }[] = [], rects: number[][] = [];
      let alpha = 1, dash: number[] = [];
      const saved: { alpha: number; dash: number[] }[] = [];
      const context = {
        save() { saved.push({ alpha, dash }); }, restore() { ({ alpha, dash } = saved.pop()!); },
        get globalAlpha() { return alpha; }, set globalAlpha(value: number) { alpha = value; },
        setLineDash(value: number[]) { dash = value; },
        rect(...args: number[]) { rects.push(args); }, clip() {}, beginPath() {}, closePath() {}, moveTo() {}, lineTo() {},
        fill() { styles.push({ alpha, dash }); }, stroke() {},
      } as unknown as CanvasRenderingContext2D;
      fixture.chart!.layers[0].draw(context, frame, { surface: '#101820' } as ChartTokens, { playhead: null });
      assert.ok(styles.some(style => style.alpha === 1 && !style.dash.length), 'exact regions retain ordinary band styling');
      assert.ok(styles.some(style => style.alpha === 0.55 && style.dash.join(',') === '4,4'), 'approximate bands draw through the shared faded dashed pass');
      const union = [frame.toX(60), frame.plotTop, frame.toX(66) - frame.toX(60), frame.plotHeight];
      assert.equal(rects.filter(rect => rect.every((value, index) => value === union[index])).length, 1, 'overlapping per-party spans clip as one approximate region');
      assert.equal(alpha, 1); assert.deepEqual(dash, []); assert.equal(saved.length, 0, 'shared clipping restores the canvas');
    };
    await settle();
    assert.ok(fixture.chart, 'the mounted party state must publish before reaching a chart view');
    fixture.chart.onSummaryView!({ from: 0, to: 1000, width: 18 }); await settle();
    const clusters = fixture.chart!.markSummaries!.flatMap(summary => summary.clusters);
    assert.deepEqual(clusters.map(cluster => [cluster.tag, cluster.firstHour]), [['ruling', 40], ['election', 40], ['native', 60], ['tail', 90]],
      'prepared census clusters and both native government streams share the dense chart');
    assert.equal(fixture.chart!.badges, undefined, 'dense census rows are not separately reclustered');
    await t.test('dense popularity draws approximate spans through the shared clipping and styling', () => approximatePaint(0, 1000, 18));
    const [, census, native, supplemental] = clusters;
    fixture.chart!.onMarkHover!(census);
    assert.match(await settle(), /aria-label="Party events"/);
    assert.ok(fixture.chart!.markTooltip!(census)?.rows.some(row => row.label.includes('Election held')));
    fixture.chart!.onMarkHover!(native);
    const both = await settle();
    assert.match(both, /aria-label="Party events"/); assert.match(both, /aria-label="Government changes"/);
    assert.ok(fixture.chart!.markTooltip!(native)?.rows.some(row => row.value === 'Coup'));
    const requestsBeforeTail = fixture.requests.length;
    fixture.chart!.onMarkHover!(supplemental); await settle();
    assert.ok(fixture.chart!.markTooltip!(supplemental)?.rows.some(row => row.value === 'Civil war'));
    assert.equal(fixture.requests.length, requestsBeforeTail, 'tail event pages read the bounded supplemental rows');
    fixture.chart!.onNavigateHour!('ArrowRight', 100, [0, 1000], null);
    await settle(); assert.equal(fixture.chart!.keyboardHour, undefined, 'a party reply cannot move the shared cursor before governments reply');
    assert.ok(deferred); deferred({ hour: governmentNext }); await settle();
    assert.equal(fixture.chart!.keyboardHour, 102, 'the nearer government change wins');
    deferGovernment = false; governmentNext = 120;
    fixture.chart!.onNavigateHour!('ArrowRight', 102, [0, 1000], null); await settle();
    assert.equal(fixture.chart!.keyboardHour, 107, 'the nearer party event wins');
    partyNext = 110; governmentNext = 115;
    fixture.chart!.onNavigateHour!('ArrowLeft', 130, [0, 1000], null); await settle();
    assert.equal(fixture.chart!.keyboardHour, 115, 'backward navigation chooses the larger candidate');
    assert.ok(fixture.requests.some(request => request.product === 'party-history' && request.params.mode === 'next'));
    assert.ok(fixture.requests.some(request => request.product === 'politics-ledger' && request.params.navigation));
    fixture.chart!.onSummaryView!({ from: 0, to: 299, width: 200 }); await settle();
    assert.deepEqual(fixture.chart!.markSummaries!.flatMap(summary => summary.clusters.map(cluster => cluster.tag)), ['ruling', 'election', 'native', 'tail'],
      'already-returned exact party clusters survive alongside dense government marks at the precision boundary');
    fixture.chart!.onSummaryView!({ from: 50, to: 80, width: 200 }); await settle();
    assert.equal(fixture.chart!.markSummaries, undefined, 'exact badges must reach TimeChart instead of an empty prepared mark list');
    assert.deepEqual(fixture.chart!.badges!.map(badge => badge.hour), [60, 80], 'the native cause replaces only its exactly matching census marker at supplying hour 64');
    assert.ok(fixture.chart!.badgesAtHour!(60).some(badge => badge.label.includes('Coup')), 'reached native badges survive party tooltip routing');
    await t.test('exact popularity draws the union of per-party approximate spans through the shared helper', () => approximatePaint(50, 80, 200));
    props.widget.countries = { kind: 'tags', tags: ['next owner'] };
    await settle(); assert.equal(fixture.chart!.keyboardHour, undefined, 'the old owner cannot keep its keyboard pick');
  } finally {
    for (const slot of fixture.slots) slot.cleanup?.();
    await vite.close(); delete host.__politicsHook; delete host.__politicsChart;
  }
});
