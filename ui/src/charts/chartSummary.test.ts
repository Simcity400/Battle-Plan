import assert from 'node:assert/strict';
import test from 'node:test';
import { binAt, binsLabel, chartSummaryFrom, drawApproximateSpans, drawLineSummary, pickSummaryLines, summaryExtent, summaryGeometry } from './chartSummary.ts';
import { wireSummary } from './chartSummary.fixture.ts';
import { dayLabel, drawBands, drawLine, spanLabel } from './draw.ts';
import { pickLines } from './linePick.ts';
import { extentOf } from './series.ts';
import type { Run } from './series.ts';
import type { ChartBucket, ChartSummary } from './chartSummary.ts';
import { plotFrame } from './frame.ts';
import type { ChartTokens } from './tokens.ts';

const frame = plotFrame(120, 100, { left: 10, right: 10, top: 10, bottom: 10 }, [0, 50_000], [-10, 10], false);
const tokens = { lineWidth: { series: 2 } } as ChartTokens;
const bucket: ChartBucket = { firstHour: 0, lastHour: 49_999, firstValue: 2, lastValue: 1, lastValueHour: 30_000,
  minHour: 20_000, minValue: -9, maxHour: 10_000, maxValue: 8, firstBreak: false, hasBreak: false };
function canvas() {
  const paths: number[][][] = [], dots: number[][] = [];
  let current: number[][] = [];
  return { paths, dots, context: {
    save() {}, restore() {}, beginPath() { current = []; },
    moveTo(x: number, y: number) { current.push([x, y]); }, lineTo(x: number, y: number) { current.push([x, y]); },
    stroke() { if (current.length) paths.push(current); }, arc(x: number, y: number) { dots.push([x, y]); }, fill() {},
  } as unknown as CanvasRenderingContext2D };
}
const summary: ChartSummary = { from: 0, to: 50_000, width: 100, buckets: [bucket] };

test('summary picks require exact known actors and preserve overlaps and gaps', () => {
  const lines = ['A', 'B'].map(key => ({ key, summary }));
  const context = { frame, until: null, point: { x: frame.toX(40_000), y: frame.toY(1) } };
  assert.deepEqual(pickSummaryLines(lines, 40_000, new Set(), context), []);
  assert.deepEqual(pickSummaryLines(lines, 40_000, new Set(['A']), context), ['A']);
  assert.deepEqual(pickSummaryLines(lines, 40_000, new Set(['A', 'B']), context), ['A', 'B']);
  const broken = { ...summary, buckets: [{ ...bucket, hasBreak: true }] };
  assert.deepEqual(pickSummaryLines([{ key: 'A', summary: broken }], 40_000, new Set(['A']), context), []);
  assert.deepEqual(pickSummaryLines(lines, 40_000, new Set(['A']), { ...context, until: 30_000 }), []);
});

test('summary draws chronological extrema with held direction using width-sized geometry', () => {
  const { context, paths } = canvas();
  const end = drawLineSummary(context, frame, tokens, summary, '#123', true);
  assert.deepEqual(end, { x: frame.toX(50_000), y: frame.toY(1) });
  const path = paths[0];
  const peak = path.findIndex(point => point[0] === frame.toX(10_000) && point[1] === frame.toY(8));
  const dip = path.findIndex(point => point[0] === frame.toX(20_000) && point[1] === frame.toY(-9));
  assert.ok(peak > 0 && dip > peak);
  assert.deepEqual(path[peak - 1], [frame.toX(10_000), frame.toY(2)], 'rise is vertical at its recorded change');
  assert.deepEqual(path[dip - 1], [frame.toX(20_000), frame.toY(8)], 'fall is vertical at its recorded change');
  assert.ok(path.some(point => point[0] === frame.toX(30_000) && point[1] === frame.toY(1)), 'last held change keeps its start hour');
  assert.ok(path.length <= 10, '50,000 hours never become 50,000 visible steps');
});

function recordingCanvas() {
  const commands: unknown[][] = [];
  const context = new Proxy({}, {
    get: (_target, key) => (...args: unknown[]) => commands.push([key, ...args]),
    set: (_target, key, value) => { commands.push([key, value]); return true; },
  }) as CanvasRenderingContext2D;
  return { context, commands };
}

const exactSource: Run[] = [
  { firstHour: -5, lastHour: 3, value: 20, breakBefore: true },
  { firstHour: 4, lastHour: 6, value: 8, breakBefore: false },
  { firstHour: 7, lastHour: 8, value: 8, breakBefore: true },
  { firstHour: 10, lastHour: 10, value: -3, breakBefore: true },
  { firstHour: 12, lastHour: 40, value: 4, breakBefore: true },
];
function hourlySummary(runs: readonly Run[], from: number, to: number, width: number): ChartSummary {
  const visible = runs.filter(run => run.lastHour >= Math.floor(from) && run.firstHour <= to);
  return { from, to, width, runs: visible, buckets: Array.from({ length: Math.floor(to) - Math.floor(from) + 1 }, (_, index) => {
    const hour = Math.floor(from) + index, run = visible.find(run => run.firstHour <= hour && hour <= run.lastHour);
    return run ? { firstHour: hour, lastHour: hour, firstValue: run.value, lastValue: run.value, lastValueHour: hour,
      minHour: hour, minValue: run.value, maxHour: hour, maxValue: run.value,
      firstBreak: hour === run.firstHour && !!run.breakBefore, hasBreak: false } : null;
  }) };
}

test('exact summaries paint the identical legacy canvas stream including gaps, breaks and fractional edges', () => {
  for (const log of [false, true]) for (const held of [false, true]) for (const until of [null, 5.5, 9.25, 10, 18.5]) {
    const frame = plotFrame(360, 200, { left: 10, right: 10, top: 10, bottom: 10 }, [1.25, 20.5], log ? [1, 30] : [-5, 30], log);
    const summary = hourlySummary(exactSource, frame.x[0], until ?? frame.x[1], Math.ceil(frame.plotWidth));
    for (const options of [{}, { area: true }, { area: true, dash: [6, 3], endDot: false }]) {
      const legacy = recordingCanvas(), prepared = recordingCanvas();
      const expected = drawLine(legacy.context, frame, tokens, exactSource, '#123', { ...options, held, until: until ?? undefined });
      const actual = drawLineSummary(prepared.context, frame, tokens, summary, '#123', held, until, options);
      assert.deepEqual(actual, expected, `end at ${until}, held=${held}`);
      assert.deepEqual(prepared.commands, legacy.commands, `commands at ${until}, held=${held}, log=${log}`);
    }
  }
});

test('exact summary extents use the same inclusive hour bounds as legacy runs', async () => {
  const { exactRuns, extentOfSummary } = await import('./chartSummary.ts');
  const summary = hourlySummary(exactSource, 1.25, 18.5, 340);
  assert.deepEqual(exactRuns(summary), exactSource);
  for (const [from, to] of [[1.25, 18.5], [4.25, 6.5], [9, 9.5], [10.1, 10.9]])
    assert.deepEqual(extentOfSummary(summary, from, to), extentOf(exactSource, from, to));
  assert.equal(exactRuns({ ...summary, runs: undefined }), null, 'dense summaries do not claim supplying runs');
});

test('dense summary extents keep only the buckets that reach the hours asked for', async () => {
  const { extentOfSummary } = await import('./chartSummary.ts');
  const dense = (firstHour: number, lastHour: number, low: number, high: number): ChartBucket => ({ firstHour, lastHour, firstValue: low, lastValue: low,
    lastValueHour: firstHour, minHour: firstHour, minValue: low, maxHour: lastHour, maxValue: high, firstBreak: false, hasBreak: false });
  const retained: ChartSummary = { from: 100, to: 2000, width: 2, buckets: [dense(100, 999, 5, 900), null, dense(1000, 2000, 8, 12)] };
  assert.deepEqual(extentOfSummary(retained), [5, 900]);
  assert.deepEqual(extentOfSummary(retained, 1000, 2200), [8, 12], 'a peak wholly before the view no longer fits its axis');
  assert.deepEqual(extentOfSummary(retained, 999.5, 2200), [5, 900], 'a bucket partly in view keeps its whole envelope');
  assert.deepEqual(extentOfSummary(retained, 50, 999), [5, 900]);
  assert.equal(extentOfSummary(retained, 2001, 2200), null);
});

test('exact summary picks follow legacy curves and held paths with overlaps, gaps and clipping', () => {
  const frame = plotFrame(360, 200, { left: 10, right: 10, top: 10, bottom: 10 }, [1.25, 20.5], [-5, 30], false);
  for (const held of [false, true]) for (const until of [null, 5.5, 9.25, 18.5]) {
    const lines = [exactSource, exactSource.map(run => ({ ...run, value: run.value + 2 })), exactSource].map((runs, index) => ({ key: String(index), runs, held }));
    const summaries = lines.map(line => ({ ...line, summary: hourlySummary(line.runs, frame.x[0], until ?? frame.x[1], Math.ceil(frame.plotWidth)) }));
    for (const hour of [2, 3, 4, 5, 7, 9, 10, 12, 18]) for (const point of [undefined, { x: frame.toX(hour + 0.4), y: frame.toY(18) }, { x: frame.toX(hour), y: frame.toY(8) }]) {
      const context = { frame, until, point };
      assert.deepEqual(pickSummaryLines(summaries, hour, new Set(lines.map(line => line.key)), context), pickLines(lines, hour, context));
      assert.deepEqual(pickSummaryLines(summaries, hour, new Set(), context), []);
    }
  }
});

test('gaps and identity breaks never become a connecting summary stroke', () => {
  const { context, paths, dots } = canvas();
  drawLineSummary(context, frame, tokens, { ...summary, buckets: [{ ...bucket, hasBreak: true }] }, '#123');
  assert.deepEqual(paths, []);
  assert.ok(dots.some(point => point[1] === frame.toY(-9)));
  assert.ok(dots.some(point => point[1] === frame.toY(8)));
});

test('resize, playback or out-of-view replies cannot paint geometry at a new owner range', () => {
  const { context, paths, dots } = canvas();
  assert.equal(drawLineSummary(context, frame, tokens, { ...summary, to: 50_001 }, '#123'), null);
  assert.equal(drawLineSummary(context, frame, tokens, { ...summary, from: 50_001, to: 60_000 }, '#123'), null);
  assert.equal(drawLineSummary(context, frame, tokens, { ...summary, width: 99 }, '#123'), null);
  assert.equal(drawLineSummary(context, frame, tokens, summary, '#123', true, 100), null);
  assert.deepEqual(paths, []); assert.deepEqual(dots, []);
});

import { compactStackExtent, forEachCompactStackSegment } from "./stack.ts";
import { heldStackSegment } from "./race/countryRace.ts";
import { summaryStack } from "./chartSummary.ts";

const stackBucket: ChartBucket = { firstHour: 0,lastHour: 31,firstValue: 2,lastValue: 3,lastValueHour: 30,
  minHour: 9,minValue: -7,maxHour: 21,maxValue: 11,firstBreak: false,hasBreak: false };
const stackSummary = (bucket: ChartBucket | null): ChartSummary => ({ from: 0,to: 31,width: 1,buckets: [bucket] });
const stackTotal = stackSummary({ ...stackBucket,firstValue: 5,lastValue: 5,minValue: 5,maxValue: 5 });
test('D-12 retains all 5000 series extrema on a shared bounded grid through the existing signed renderer', () => {
  const parts=Array.from({length:5000},(_,index)=>stackSummary({...stackBucket,minHour:1+index%14,maxHour:16+index%14}));
  const dense=summaryStack(parts,stackTotal);
  assert.ok(dense.model.hours.length<=15,'grid size is independent of the number of extrema dates');
  assert.equal(dense.model.bands.length,0,'edges are allocated only for emitted screen points');
  const values=parts.map(()=>new Set<number>());
  forEachCompactStackSegment(dense.model,0,31,0,segment=> {
    assert.equal(segment.bands.length,5001);
    segment.bands.slice(0,-1).forEach((band,index)=> {
      band.low.forEach((low,point)=>values[index].add(band.high[point]<=0 && low<0 ? low-band.high[point] : band.high[point]-low));
    });
  },dense.sample);
  for (const kept of values) assert.deepEqual(kept,new Set([2,-7,11,3]));
  const one=summaryStack([stackSummary(stackBucket)],stackTotal);
  assert.deepEqual(compactStackExtent(one.model,0,31),[-7,12]);
});

test('the landed stack renderer isolates broken buckets and never connects a missing column', () => {
  const frame=plotFrame(320,200,{left:0,right:0,top:0,bottom:0},[0,31],[-10,20],false);
  const calls:unknown[][]=[];
  const ctx=new Proxy({},{get:(_target,key)=>(...args:unknown[])=>calls.push([key,...args]),set:()=>true}) as CanvasRenderingContext2D;
  const broken=summaryStack([stackSummary({...stackBucket,hasBreak:true})],stackTotal);
  let pieces=0;
  forEachCompactStackSegment(broken.model,0,31,0,piece=> {
    pieces++;assert.ok(piece.hours.at(-1)!-piece.hours[0]<=1);
    const segment=heldStackSegment(piece);
    drawBands(ctx,frame,{lineWidth:{series:2}} as ChartTokens,segment.hours,segment.bands,['#123','#456'],segment);
  },broken.sample);
  assert.equal(pieces,5);assert.ok(calls.some(call=>call[0]==='fill'));
  const shift=(value:ChartBucket,offset:number):ChartBucket=>({...value,firstHour:value.firstHour+offset,lastHour:value.lastHour+offset,
    minHour:value.minHour+offset,maxHour:value.maxHour+offset,lastValueHour:value.lastValueHour+offset});
  const view:ChartSummary={from:0,to:95,width:3,buckets:[stackBucket,null,shift(stackBucket,64)]};
  const dense=summaryStack([view],{...view,buckets:[stackTotal.buckets[0],shift(stackTotal.buckets[0]!,32),shift(stackTotal.buckets[0]!,64)]});
  forEachCompactStackSegment(dense.model,0,95,0,piece=>assert.ok(piece.hours.at(-1)!<=32 || piece.hours[0]>=64),dense.sample);
});




test('an exact stack keeps the identical legacy draw stream through the shared helper', () => {
  const direct = recordingCanvas(), shared = recordingCanvas();
  const paint = (ctx: CanvasRenderingContext2D) => drawBands(ctx, frame, tokens, [0, 50_000], [{ low: [0, -2], high: [3, 0] }], ['#123']);
  paint(direct.context);
  drawApproximateSpans(shared.context, frame, [], null, approximate => { assert.equal(approximate, false); paint(shared.context); });
  assert.deepEqual(shared.commands, direct.commands);
});

test('a stack without a total retains named percentages and signed extrema without a residual band', () => {
  const constant=stackSummary({...stackBucket,firstValue:35,lastValue:35,minValue:35,maxValue:35});
  const dense=summaryStack([stackSummary(stackBucket),constant]);
  const kept=new Set<number>();
  forEachCompactStackSegment(dense.model,0,31,0,piece=> {
    assert.equal(piece.bands.length,2,'every band is a supplied series');
    piece.bands[0].low.forEach((low,index)=> {const high=piece.bands[0].high[index];kept.add(low<0 ? low-high : high-low);});
  },dense.sample);
  assert.deepEqual(kept,new Set([2,-7,11,3]));
  assert.deepEqual(compactStackExtent(dense.model,0,31),[-7,46]);
  const percentages=summaryStack([stackSummary({...stackBucket,firstValue:40,lastValue:40,minValue:40,maxValue:40}),constant]);
  for (const hour of percentages.model.hours) if (percentages.model.bounds[percentages.model.hours.indexOf(hour)]) {
    assert.equal(percentages.sample(hour).at(-1)!.high,75,'recorded percentages are never normalized');
  }
  const unknown=summaryStack([stackSummary(null),constant]);
  assert.deepEqual(unknown.model.hours,[]);
});

test('a dense stack draws a column only over the hours every part was recorded', () => {
  const part = (firstHour: number, lastHour: number, value: number) => stackSummary({ firstHour, lastHour, firstValue: value, lastValue: value,
    lastValueHour: lastHour, minHour: firstHour, minValue: value, maxHour: firstHour, maxValue: value, firstBreak: false, hasBreak: false });
  assert.deepEqual(summaryStack([part(0, 9, 10), part(20, 29, 5)]).model.hours, []);
  const overlap = summaryStack([part(0, 19, 10), part(10, 29, 5)]);
  const drawn = overlap.model.hours.filter((_, index) => overlap.model.bounds[index]);
  assert.ok(drawn.length > 0);
  assert.ok(drawn.every(hour => hour >= 10 && hour <= 19), `${drawn}`);
  for (const bound of overlap.model.bounds) if (bound) assert.deepEqual(bound, [0, 15]);
});

test('stack summaries carry the union of part and total approximate spans without changing geometry', () => {
  const a = stackSummary(stackBucket), b = stackSummary({ ...stackBucket, firstValue: 3, lastValue: 3, minValue: 3, maxValue: 3 });
  const exact = summaryStack([a,b],stackTotal);
  const spans = [{ firstHour: 8, lastHour: 12 }, { firstHour: 2, lastHour: 5 }];
  const stacked = summaryStack([{ ...a, approximate: spans }, { ...b, approximate: [{ firstHour: 5, lastHour: 9 }, { firstHour: 20, lastHour: 21 }] }],
    { ...stackTotal, approximate: [{ firstHour: 13, lastHour: 15 }] });
  assert.deepEqual(stacked.approximate,[{ firstHour: 2, lastHour: 15 }, { firstHour: 20, lastHour: 21 }]);
  assert.deepEqual(stacked.model,exact.model);
  for (const hour of exact.model.hours) if (exact.model.bounds[exact.model.hours.indexOf(hour)]) assert.deepEqual(stacked.sample(hour),exact.sample(hour));
  assert.deepEqual(spans,[{ firstHour: 8, lastHour: 12 }, { firstHour: 2, lastHour: 5 }], 'a summary owned by another chart is not mutated');
  assert.deepEqual(exact.approximate,[]);
});

test('a summary decodes from its bucket columns, gaps and totals in place, and refuses columns that disagree', () => {
  const later = { ...bucket, firstHour: 60_000, lastHour: 60_009, lastValueHour: 60_004, minHour: 60_001, maxHour: 60_009, firstBreak: true, hasBreak: true, absoluteTotal: '123456789012345678901234567890' };
  const sent = { from: 0, to: 70_000, width: 4, buckets: [null, { ...bucket, absoluteTotal: '7' }, null, null, later, null] };
  const wire = wireSummary(sent);
  assert.deepEqual(wire.buckets.skip, [1, 2]);
  assert.deepEqual(wire.buckets.hours.slice(5), [60_000 - 49_999, 9, 4, 1, 9]);
  assert.deepEqual(chartSummaryFrom(wire), sent);
  assert.equal(chartSummaryFrom(wire), chartSummaryFrom(wire), 'decoded once per reply');
  for (const columns of [{ slots: 2 }, { flags: [0, 4] }, { totals: ['7', '-1'] }, { hours: [...wire.buckets.hours.slice(0, 6), 3, 4, 1, 9] },
    { skip: [1] }, { values: [...wire.buckets.values.slice(0, 7), Infinity] }])
    assert.equal(chartSummaryFrom({ ...wire, buckets: { ...wire.buckets, ...columns } }), null, JSON.stringify(columns));
  assert.equal(chartSummaryFrom(sent), null, 'bucket objects are not the wire');
});

test('a frame that cannot place a value draws that column as its placed dots, never a line across it', () => {
  const frame = plotFrame(120, 100, { left: 10, right: 10, top: 10, bottom: 10 }, [0, 29], [1e-323, 1], true);
  assert.equal(frame.toY(0), Infinity, 'the log floor underflows, so zero has no place');
  const column = (first: number, low: number): ChartBucket => ({ firstHour: first, lastHour: first + 9, firstValue: 0.5, lastValue: 0.5, lastValueHour: first,
    minHour: first + 2, minValue: low, maxHour: first, maxValue: 0.5, firstBreak: false, hasBreak: false });
  const summary: ChartSummary = { from: 0, to: 29, width: 100, buckets: [column(0, 0.25), column(10, 0), column(20, 0.25)] };
  const geometry = summaryGeometry(frame, summary);
  assert.equal(geometry.paths.length, 2, 'the unplaced column joins neither neighbour');
  assert.ok(geometry.paths.flat().every(command => command.slice(1).every(value => Number.isFinite(value))));
  assert.equal(geometry.dots.length, 3, 'its placed corners stay as dots');
  const drawn = canvas();
  drawLineSummary(drawn.context, frame, tokens, summary, '#123');
  assert.equal(drawn.paths.length, 2);
  assert.ok(drawn.paths.flat(2).every(value => Number.isFinite(value)));
});

test('a tooltip of bins\' values names the days the bins span, never one hour of them; one-hour bins name the hour\'s day', () => {
  const DAY = 24, from = 43_800_000 + 400 * DAY;
  const bin = (first: number, last: number): ChartBucket => ({ firstHour: first, lastHour: last, firstValue: 1, lastValue: 2, lastValueHour: first,
    minHour: first, minValue: 1, maxHour: first, maxValue: 2, firstBreak: false, hasBreak: false });
  const wide = { from, to: from + 8 * 96 - 1, width: 8, buckets: Array.from({ length: 8 }, (_, at) => bin(from + at * 96, from + at * 96 + 95)) };
  const hovered = from + 96 + 40;
  assert.deepEqual(binAt(wide, hovered), wide.buckets[1]);
  assert.equal(binAt(wide, from - 1), null);
  assert.equal(binsLabel(hovered, [binAt(wide, hovered)]), spanLabel(from + 96, from + 96 + 95));
  assert.notEqual(binsLabel(hovered, [binAt(wide, hovered)]), dayLabel(hovered));
  assert.equal(binsLabel(hovered, [bin(from + 100, from + 150), null, bin(from + 96, from + 120)]), spanLabel(from + 96, from + 150));
  assert.equal(binsLabel(hovered, [bin(hovered, hovered), null]), dayLabel(hovered));
  assert.equal(binsLabel(hovered, []), dayLabel(hovered));
});
