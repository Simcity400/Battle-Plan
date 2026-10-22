import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { TimeChartProps } from '../../charts/TimeChart';
import { plotFrame } from '../../charts/frame.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ChartTokens } from '../../charts/tokens';
import { formatExperience } from './experienceData.ts';

test('experience widget reads only viewport summaries and reached hours while preserving exact raw values', async () => {
  type Slot = { value?: unknown; deps?: readonly unknown[]; cleanup?: () => void };
  const tag = 'opaque / country', raw = '9223372036854775807';
  const fixture = { cursor: 0, slots: [] as Slot[], effects: [] as (() => void)[], chart: undefined as TimeChartProps | undefined,
    groupError: undefined as string | undefined, groupsHeld: true,
    prefix: undefined as number | undefined,
    histories: [{tag,identities:[{slot:7,countryId:11,firstHour:0,lastHour:100}]}],
    products: [] as { product: string; active: boolean }[], queries: [] as { product: string; params: Record<string, unknown>; signal: AbortSignal }[],
    tokens: { surface: '#000', muted: '#888', lineWidth: { series: 2 }, countryColours: 'raw', categories: ['#123'] } as unknown as ChartTokens,
    styles: { known: new Map([[tag, { tag, name: tag, color: [20, 50, 80] }]]), data: { warnings: [] } } };
  const host = globalThis as typeof globalThis & { __preparedExperienceFixture?: typeof fixture; __preparedExperienceHook?: (name: string, ...args: unknown[]) => unknown };
  host.__preparedExperienceFixture = fixture;
  host.__preparedExperienceHook = (name, ...args) => {
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
    if (!('value' in slot)) slot.value = name === 'useReducer' ? args[1] : name === 'useRef' ? { current: args[0] } : args[0];
    if (name === 'useRef') return slot.value;
    return [slot.value, (next: unknown) => { slot.value = name === 'useReducer'
      ? (args[0] as (previous: unknown, action: unknown) => unknown)(slot.value, next)
      : typeof next === 'function' ? (next as (previous: unknown) => unknown)(slot.value) : next; }];
  };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, request, active) => {
      if (request) globalThis.__preparedExperienceFixture.products.push({product:request.product,active}); return {retry(){}};
    }; export const useProducts = (_client, requests, active) => {
      for (const request of requests) globalThis.__preparedExperienceFixture.products.push({product:request.product,active}); return {retry(){}};
    };`,
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({retry(){},error:globalThis.__preparedExperienceFixture.groupError,data:globalThis.__preparedExperienceFixture.groupsHeld
      ? {warnings:[],capitulations:[],countryIdentities:globalThis.__preparedExperienceFixture.histories} : undefined});`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 100; export const usePlaybackSpeed = () => 24;',
    '/dashboard/useCountryLineStyles.ts': 'export const useCountryLineStyles = () => globalThis.__preparedExperienceFixture.styles;',
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour,tag) => JSON.stringify([hour,tag]);
      export const useCountryNamesRead = (_client,_track,wanted) => new Map(wanted.map(row => [nameKey(row.hour,row.tag),row.tag]));`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt:()=>tag=>({name:tag,known:true}),revision:0});',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({color:()=>[20,50,80],flag(){},flagPending:()=>false,flagReason(){}}); export const lookPlan = (_branch,tags,hour) => ({wanted:tags.map(tag=>({tag,hour})),maxHours:128});`,
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => globalThis.__preparedExperienceFixture.tokens; export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({open(){},menu:null});',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default props => {globalThis.__preparedExperienceFixture.chart=props; return null;};',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'prepared-experience-fixture', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
      if (replacement) return replacement[1];
      if (path.endsWith('/widgets/experience/ExperienceWidget.tsx') || path.endsWith('/widgets/experience/preparedExperience.ts') || path.endsWith('/widgets/country-measure/preparedMeasure.ts'))
        return source.replace(/\b(useMemo|useCallback|useState|useReducer|useRef|useEffect)(?:<[\s\S]*?>)?\(/g,
          (_match, name) => `globalThis.__preparedExperienceHook(${JSON.stringify(name)}, `);
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/experience/ExperienceWidget.tsx');
    const source = { id: 'source', revision: '1', path: 'synthetic', metadata: { campaignKey: 'synthetic', branches: [], countries: [] } };
    const client = { preparedQuery: async (_source: unknown, product: string, params: Record<string, unknown>, { signal }: { signal: AbortSignal }) => {
      fixture.queries.push({ product, params, signal });
      if (product === 'series-next-change') {
        return { hour: 90 };
      }
      const family = params.family as {kind:string;measure:string}; assert.equal(family.kind,'country-experience'); assert.match(family.measure,/^(army|navy|air)-banked$/);
      if (product === 'series-state') return { branchId: 'branch', measure: family.measure, decimals: 0, scale:32768, dayHours:24, warnings: [],
        states: (params.reads as {tag:string;hour:number}[]).map(read => { const [tag] = JSON.parse(read.tag);
          const covered = read.hour!==50 && fixture.histories.find(history=>history.tag===tag)?.identities.some(identity=>identity.firstHour<=read.hour&&read.hour<=identity.lastHour); return ({...read,recorded:true,available:true,segmentFirstHour:read.hour===50?null:read.hour<50?0:51,readHour:read.hour===50?null:read.hour<50?0:51,
          run:!covered?null:{firstHour:read.hour<50?0:51,lastHour:read.hour<50?49:100,valueRaw:raw,slot:7,countryId:11,startsSegment:true},drop:null,
          experience:!covered?null:{firstHour:read.hour<50?0:51,lastHour:read.hour<50?49:100,readHour:read.hour<50?0:51,startsSegment:true,
            armyRaw:raw,navyRaw:raw,airRaw:raw,gain:null,gainUnknown:'noCompleteDay',change:null,changeUnknown:'noMatchingRead'}}); }) };
      assert.equal(product, 'series-summary');
      if (Number(params.to) > 100) return null;
      const width = params.width as number;
      const prefix = fixture.prefix;
      return { branchId: 'branch', measure: family.measure, decimals: 0, warnings: [],
        ...(prefix === undefined ? {} : { coverage: {firstHour:0,lastHour:prefix,complete:false},
          pending: [{firstHour:0,lastHour:49},{firstHour:51,lastHour:100}]
            .map(span => ({...span,firstHour:Math.max(span.firstHour,prefix+1)})).filter(span => span.firstHour<=span.lastHour) }),
        series: (params.tags as string[]).map(tag => { const spans = (params.within as Record<string,{firstHour:number;lastHour:number}[]>)[tag]; return ({ tag,
        summary: { from: params.from, to: params.to, width,
          runs: [{ firstHour: 0, lastHour: 49, value: Number(raw), breakBefore: true },
            { firstHour: 51, lastHour: 100, value: Number(raw), breakBefore: true }]
            .map(run => ({...run,lastHour:Math.min(run.lastHour,prefix??100)}))
            .filter(run=>run.firstHour<=run.lastHour && spans.some(span=>span.firstHour<=run.firstHour && run.lastHour<=span.lastHour)),
          buckets: Array.from({ length: Math.floor(Number(params.to)) - Math.floor(Number(params.from)) + 1 }, (_, index) => {
            const hour = Math.floor(Number(params.from)) + index;
            return hour > (prefix??100) || hour === 50 || !spans.some(span=>span.firstHour<=hour&&hour<=span.lastHour) ? null : { firstHour: hour, lastHour: hour,
            firstValue: Number(raw), lastValue: Number(raw), lastValueHour: hour,
            minHour: hour, minValue: Number(raw), maxHour: hour, maxValue: Number(raw),
            firstBreak: hour === 0 || hour === 51, hasBreak: false }; }) }, marks: null }); }) };
    }, preparationStatus: async (_request: object, signal: AbortSignal) => new Promise((_resolve, reject) => {
      signal.addEventListener('abort', () => reject(new DOMException('Cancelled', 'AbortError')), { once: true });
    }) };
    const props = {widget:{id:'experience',kind:'experience',trackIds:['track'],countries:{kind:'tags',tags:[tag]},time:{mode:'campaign'},
      options:{endLabels:false,measure:'banked',army:true,navy:false,air:false}},
      runtime:{client,tracks:[{id:'track',axisOffset:0,source,branch:{id:'branch',firstHour:0,lastHour:100}}],clock:{getHour:()=>100,getSnapshot:()=>({playing:false,hoursPerSecond:24})}},
      active:true,headerSlot:null,onDiagnostics(){}} as unknown as WidgetProps;
    const render = () => { fixture.cursor = 0; fixture.effects = []; fixture.chart = undefined;
      const html = renderToStaticMarkup(createElement(Widget, props)); for (const effect of fixture.effects) effect(); return html; };
    const settle = async () => { await new Promise(resolve => setImmediate(resolve)); render(); await new Promise(resolve => setImmediate(resolve)); render(); };
    render(); await settle();
    assert.ok(fixture.chart);
    assert.ok(fixture.products.every(request => !request.active), 'no full experience history starts on mount');
    fixture.chart.onSummaryView!({ from: 0, to: 100, width: 100 }); render(); await settle();
    const frame = plotFrame(100, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 2e14], false);
    let operations = 0;
    let curves = 0;
    const context = { save(){},restore(){},beginPath(){},moveTo(){operations++;},lineTo(){operations++;},bezierCurveTo(){operations++; curves++;},stroke(){},fill(){},arc(){},closePath(){},setLineDash(){} } as unknown as CanvasRenderingContext2D;
    fixture.chart!.layers[0].draw(context, frame, fixture.tokens, { playhead: null });
    assert.ok(operations > 0 && operations <= 100 * 27, 'line and area paths are bounded by viewport width');
    assert.ok(curves > 0, 'exact experience uses the legacy cubic path');
    assert.deepEqual(fixture.chart!.layers[0].extent?.([0, 100]), [Number(raw)/32768, Number(raw)/32768]);
    fixture.chart!.onHover!(90); render(); await settle();
    const tooltip = fixture.chart!.tooltip!(90, frame, []);
    assert.equal(tooltip!.rows[0].value, formatExperience(raw,32768), 'geometry f64 never supplies the exact tooltip');
    fixture.chart!.onHover!(50); render(); await settle();
    assert.equal(fixture.chart!.tooltip!(50, frame, [])!.rows[0].value, 'Not recorded at this time');
    fixture.chart!.onNavigateHour!('Home',null,[0,100],null); await settle();
    assert.equal(fixture.chart!.keyboardHour,90);
    assert.ok(fixture.queries.every(query=>query.product!=='series-state'||(query.params.reads as unknown[]).length<=3),'exact reads stay bounded to the head, hover and line ends');
    fixture.histories = [{tag,identities:[{slot:7,countryId:11,firstHour:0,lastHour:49}]},
      {tag:'renamed',identities:[{slot:7,countryId:11,firstHour:51,lastHour:100}]},
      {tag:'unrelated',identities:[{slot:8,countryId:12,firstHour:51,lastHour:100}]}];
    render(); await settle();
    assert.equal(fixture.chart!.layers.length,1,'aliases stay one native holder');
    assert.match(fixture.chart!.label,/renamed/,'the head uses the current alias');
    const aliases = new Set(fixture.queries.filter(query=>query.product==='series-state').flatMap(query=>(query.params.reads as {tag:string}[]).map(read=>JSON.parse(read.tag)[0])));
    assert.ok(aliases.has('renamed')); assert.ok(!aliases.has('unrelated'),'unrelated reuse never enters the query');
    // Exact reached amounts remain usable while history is empty or partial.
    Object.assign(props.widget.options!, {navy:true,air:true});
    for (const [prefix, width, expected] of [
      [-1, 101, [[0,50],[51,49]]], [39, 102, [[40,10],[51,49]]],
    ] as const) {
      fixture.prefix = prefix;
      fixture.chart!.onSummaryView!({from:0,to:100,width}); render(); await settle();
      assert.match(render(), /Still being prepared/);
      assert.equal(fixture.chart!.tooltip!(90,frame,[])!.rows[0].value,formatExperience(raw,32768));
      const bands: number[][] = [];
      const bandContext = {save(){},restore(){},fillRect(x:number,_y:number,width:number){bands.push([x,width]);}} as unknown as CanvasRenderingContext2D;
      fixture.chart!.layers[0].draw(bandContext,frame,fixture.tokens,{playhead:null});
      assert.deepEqual(bands, expected, 'pending hours are banded once, excluding the recorded gap');
    }
    Object.assign(props.widget.options!, {navy:false,air:false});
    fixture.prefix = undefined;
    fixture.chart!.onSummaryView!({from:0,to:100,width:103}); render(); await settle();
    assert.doesNotMatch(render(), /Still being prepared/);
    assert.equal(fixture.chart!.layers.length,1,'the band disappears with exact history');
    fixture.groupError='Identity catalogue unavailable'; fixture.products=[];
    const held = render();
    assert.ok(fixture.chart, 'the accepted chart stays drawn');
    assert.ok(fixture.products.every(request=>request.product!=='country-experience'||!request.active),'retained group history keeps the prepared reader');
    assert.match(held, /data-detail="Identity catalogue unavailable"/);
    assert.match(held, />Retry</);
    fixture.groupsHeld=false; fixture.products=[]; render();
    assert.ok(fixture.products.some(request=>request.product==='country-experience'&&request.active),'missing marker metadata does not hide independent experience');
  } finally {
    for (const slot of fixture.slots) slot.cleanup?.();
    await vite.close(); delete host.__preparedExperienceFixture; delete host.__preparedExperienceHook;
  }
});
