import test from 'node:test';
import assert from 'node:assert/strict';
import { plotFrame } from '../../charts/frame.ts';
import { fitTooltip } from '../../charts/tooltip.ts';
import { lookPlan } from '../../dashboard/countryLooks.ts';
import { sampledLooks } from '../../dashboard/looks.ts';
import { ChartPicks } from '../../charts/pick.ts';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState.ts';
import { NumericReply } from '../../dashboard/numericReply.ts';
import { createWidget, resolveWindow } from '../../dashboard/model.ts';
import type { BoundTrack } from '../../dashboard/model';
import type { CountryStabilityReply, StabilityPartsReply } from '../../generated/dashboard';
import '../../widgets/definitions.ts';
import { chooseStabilityActor, pickStabilityLine, pinStabilityDrill, stabilityNumbers, stabilityPartsNumbers,
  stabilityPickAction, stabilityPlotSeries, stabilityLookSegments, stabilityCountryParts } from './stabilityDrillData.ts';
import type { StabilityLine } from './stabilityDrillData';

const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 100], false);
const line = (tag: string, measure: StabilityLine['measure'], value: number): StabilityLine => ({ tag, measure,
  runs: [{ firstHour: 0, lastHour: 100, value }] });
const pointer = (value: number, hour = 49) => ({ frame, until: null, point: { x: frame.toX(hour), y: frame.toY(value) } });
const reply = (count = 2): CountryStabilityReply => ({ branchId: 'branch', decimals: 5, warnings: [],
  series: Array.from({ length: count }, (_, index) => ({ tag: `native_${index}`, recorded: true, breakdown: null,
    stability: [{ firstHour: 0, lastHour: 100, valueRaw: '50000', startsSegment: true }],
    warSupport: [{ firstHour: 0, lastHour: 100, valueRaw: '80000', startsSegment: true }], maxCommandPower: [], carried: [],
  })) });

test('a comparison click chooses its nearest drawn actor and measure without using raw values', () => {
  const lines = [line('a:warSupport', 'stability', 10), line('b', 'stability', 90), line('b', 'warSupport', 40)];
  const picked = pickStabilityLine(lines, 49, pointer(40))!;
  assert.equal(picked.tag, 'b');
  assert.equal(picked.measure, 'warSupport');
  assert.equal(picked.hour, 49);
  const gliding = [{ ...line('raw nearest', 'stability', 0) }, { ...line('drawn nearest', 'warSupport', 0), runs: [
    { firstHour: 0, lastHour: 49, value: 0 }, { firstHour: 50, lastHour: 100, value: 100 }] }];
  assert.equal(pickStabilityLine(gliding, 49, pointer(40))!.tag, 'drawn nearest');
});

test('overlap and keyboard choices retain actor-specific measure and held hour while playback advances', () => {
  const lines = [line('a', 'stability', 80), line('b', 'warSupport', 80)];
  for (const context of [pointer(80), { frame, until: 49 }]) {
    const picked = pickStabilityLine(lines, 49, context)!;
    assert.equal(picked.tag, null);
    assert.deepEqual(picked.candidates, ['a', 'b']);
    let drill = reduceDrill(CLOSED, stabilityPickAction(picked, false, 'follow', true, 50));
    assert.equal(drill.follow, false);
    assert.equal(drillHour(drill, 100), 49);
    assert.equal(chooseStabilityActor(picked, 'missing'), picked);
    const chosen = chooseStabilityActor(picked, 'b');
    assert.equal(chosen.measure, 'warSupport');
    assert.equal(chosen.tag, 'b');
    drill = reduceDrill(drill, stabilityPickAction(chosen, true, 'follow', true, 100));
    assert.equal(drillHour(drill, 100), 49);
  }
});

test('hidden measures, gaps, clipped and future curves and absent frames never supply actors', () => {
  const displayed = [line('shown', 'warSupport', 80), line('clipped', 'stability', 200),
    { ...line('gap', 'warSupport', 80), runs: [{ firstHour: 0, lastHour: 10, value: 80 }] }, line('missing', 'stability', NaN)];
  assert.deepEqual(pickStabilityLine(displayed, 49, { frame, until: 49 })!.candidates, ['shown']);
  assert.equal(pickStabilityLine(displayed, 50, { frame, until: 49 }), null);
  assert.equal(pickStabilityLine([line('single', 'stability', 80)], 49), null);
  assert.equal(pickStabilityLine([], 49, pointer(80)), null);
});

test('Campaign, Pin and Range opens hold the drawn hour even while the global clock is playing', () => {
  const pick = pickStabilityLine([line('actor', 'warSupport', 80)], 20, pointer(80, 20))!;
  for (const mode of ['campaign', 'pin', 'range'] as const) {
    const drill = reduceDrill(CLOSED, stabilityPickAction(pick, false, mode, true, 100));
    assert.equal(drill.follow, false);
    assert.equal(drillHour(drill, 100), 20);
    const following = reduceDrill(drill, { type: 'follow' });
    assert.equal(drillHour(following, 80), 80, 'explicit Follow reads the actual aligned source clock');
  }
  assert.equal(reduceDrill(CLOSED, stabilityPickAction(pick, false, 'follow', true, 100)).follow, true);
});

test('Pin preserves picked actor, measure and branch-relative hour; explicit Follow makes a Follow widget', () => {
  const widget = createWidget('stability', 'track', 'stability-test');
  widget.countries = { kind: 'tags', tags: ['a', 'b'] };
  const pin = pinStabilityDrill(widget, { id: 'warSupport', label: 'War support parts' }, 'track', 1000, 'b', 1020, false);
  assert.deepEqual(pin.countries, { kind: 'tags', tags: ['b'] });
  assert.deepEqual(pin.time, { mode: 'pin', windows: { track: { firstOffset: 20, lastOffset: 20 } } });
  assert.equal(pin.options?.show, 'warSupport');
  const track = { id: 'track', axisOffset: 900, branch: { firstHour: 1000, lastHour: 2000 } } as BoundTrack;
  assert.deepEqual(resolveWindow(pin, track, [track], 500), { firstHour: 1020, lastHour: 1020 });
  const live = pinStabilityDrill(widget, { id: 'stability', label: 'Stability parts' }, 'track', 1000, 'b', 1020, true);
  assert.equal(live.time.mode, 'follow');
  assert.equal(resolveWindow(live, track, [track], 200)!.lastHour, 1100);
});

test('numeric history survives content pending, metadata callbacks and 500ms at 10Hz but changed contexts cancel picks', () => {
  const retained = new NumericReply<CountryStabilityReply>();
  const first = reply();
  const numbers = retained.update('source/branch/a-b', first, stabilityNumbers);
  const refreshed = { ...reply(), warnings: ['Changed content names'], series: reply().series.map(row => ({ ...row, maxCommandPower: [{ firstHour: 0, lastHour: 100, valueRaw: '123', startsSegment: true }] })) };
  assert.equal(retained.update('source/branch/a-b', undefined, stabilityNumbers), numbers);
  assert.equal(retained.update('source/branch/a-b', refreshed, stabilityNumbers), numbers);
  const owner = { scope: 'source/branch/selection/time/visible measures', numbers };
  const picks = new ChartPicks();
  const delivered: unknown[] = [];
  const callback = (hour: number, context?: Parameters<typeof pickStabilityLine>[2]) => delivered.push(pickStabilityLine([
    line('a', 'stability', 10), line('b', 'warSupport', 80)], hour, context));
  const scope = { owner, extent: [0, 100] as const, branchStart: 0, active: true, onPick: callback };
  picks.update(scope);
  const clicked = picks.drawn(pointer(80), callback);
  for (let elapsed = 100; elapsed < 500; elapsed += 100) picks.update({ ...scope,
    onPick: () => { throw new Error('Metadata or playback replaced the drawn callback'); } });
  assert.equal(picks.pick(clicked, 49), true);
  assert.equal((delivered[0] as { tag: string }).tag, 'b');
  for (const changed of ['source', 'selection', 'branch', 'time range', 'hidden measure', 'numeric operands']) {
    const before = picks.drawn({ frame, until: 49 }, callback);
    picks.update({ ...scope, owner: { scope: changed, numbers } });
    picks.update(scope);
    assert.equal(picks.pick(before, 49), false, 'A to B to A cannot revive an old frame');
  }
});

test('parts retain native inventory, gaps and quantities across label-only refreshes', () => {
  const parts: StabilityPartsReply = { branchId: 'branch', tag: 'actor', decimals: 5, warnings: [],
    stability: { observed: [{ firstHour: 0, lastHour: 100 }], parts: [{ id: 'native source', term: null, line: null,
      runs: [{ firstHour: 0, lastHour: 100, valueRaw: '5000', startsSegment: true }] }] }, warSupport: { observed: [], parts: [] } };
  const store = new NumericReply<StabilityPartsReply>();
  const numeric = store.update('source/branch/actor', parts, stabilityPartsNumbers);
  const lineName = { line: 1, kind: 'static', identity: 'native identity', identityId: 0, secondaryRaw: '0', valueRaw: '5000',
    text: null, label: 'New content language', group: null, groupKind: null };
  assert.equal(store.update('source/branch/actor', { ...parts, warnings: ['New name warning'], stability: {
    ...parts.stability, parts: [{ ...parts.stability.parts[0], line: lineName }] } }, stabilityPartsNumbers), numeric);
  assert.equal(store.update('source/branch/actor', undefined, stabilityPartsNumbers), numeric);
  assert.notEqual(store.update('source/branch/actor', { ...parts, stability: { ...parts.stability,
    observed: [{ firstHour: 1, lastHour: 100 }] } }, stabilityPartsNumbers), numeric);
  assert.equal(store.update('other source/branch/actor', undefined, stabilityPartsNumbers), undefined);
});

test('five thousand synthetic countries stay in plot, keyboard actor inventory and full tooltip text', () => {
  const native = reply(5000);
  const series = stabilityPlotSeries(native, null, [0, 100]);
  assert.equal(series.length, 5000);
  const picked = pickStabilityLine(series.map(row => ({ tag: row.tag, measure: 'stability' as const, runs: row.lines.stability })),
    49, { frame, until: 49 })!;
  assert.equal(picked.candidates.length, 5000);
  assert.equal(chooseStabilityActor(picked, 'native_4999').tag, 'native_4999');
  const tooltip = fitTooltip([{ title: 'Recorded actors', rows: series.map(row => ({ label: row.tag, value: '50.0%' })) }], 600);
  assert.ok(tooltip.rows.length < 5000);
  assert.ok(tooltip.spoken?.includes('native_4999'));
  assert.ok(tooltip.rows.some(row => row.label.includes('more')));
});

test('native replacements and unknown hours bound every sampled and held country look', () => {
  const native = reply(1);
  native.series[0].stability = [
    { firstHour: 0, lastHour: 10, valueRaw: '50000', startsSegment: true },
    { firstHour: 11, lastHour: 20, valueRaw: '60000', startsSegment: false },
    { firstHour: 21, lastHour: 30, valueRaw: '60000', startsSegment: true },
  ];
  native.series[0].warSupport = [
    { firstHour: 0, lastHour: 20, valueRaw: '80000', startsSegment: true },
    { firstHour: 21, lastHour: 30, valueRaw: '80000', startsSegment: true },
    { firstHour: 40, lastHour: 50, valueRaw: '80000', startsSegment: true },
  ];
  const segmentAt = stabilityLookSegments(native);
  assert.equal(segmentAt('native_0', 20), 0);
  assert.equal(segmentAt('native_0', 21), 21);
  assert.equal(segmentAt('native_0', 35), null);
  assert.equal(segmentAt('native_0', 45), 40);
  assert.equal(segmentAt('missing', 21), null);
  const branch = { firstHour: 0, lastHour: 100, gaps: [] };
  const before = sampledLooks({ looks: new Map(), segments: new Map() }, lookPlan(branch, ['native_0'], 20, segmentAt).samples,
    () => ({ style: { name: 'Prior holder', color: [1, 2, 3] }, flag: 'prior-flag' }));
  const replacement = lookPlan(branch, ['native_0'], 21, segmentAt);
  assert.equal(replacement.samples[0].hour, 21, 'the month sample cannot read the prior holder');
  const held = sampledLooks(before, replacement.samples, () => ({ style: undefined, flag: undefined }));
  assert.equal(held.looks.has('native_0'), false, 'no old name, colour or flag while replacement is pending');
  assert.equal(lookPlan(branch, ['native_0'], 35, segmentAt).samples.length, 0);
  assert.equal(lookPlan({ ...branch, gaps: [{ firstHour: 45, lastHour: 45 }] }, ['native_0'], 45, segmentAt).samples.length, 0);
});


test('stability parts join selected aliases without replacement values or an unread identity gap', () => {
  const native = (tag: string, runs: { firstHour: number; lastHour: number; valueRaw: string; startsSegment: boolean }[]): StabilityPartsReply => ({
    tag, branchId: 'b', decimals: 5, warnings: [], stability: { parts: [{ id: 'term:base', term: 'base', line: null, runs }],
      observed: runs.map(({ firstHour, lastHour }) => ({ firstHour, lastHour })) }, warSupport: { observed: [], parts: [] },
  });
  const x = native('X', [{ firstHour: 0, lastHour: 9, valueRaw: '10000', startsSegment: true },
    { firstHour: 10, lastHour: 20, valueRaw: '99999', startsSegment: true }]);
  const y = native('Y', [{ firstHour: 10, lastHour: 12, valueRaw: '20000', startsSegment: true },
    { firstHour: 14, lastHour: 20, valueRaw: '30000', startsSegment: true }]);
  const spans = new Map([['X', [{ firstHour: 0, lastHour: 9 }]], ['Y', [{ firstHour: 10, lastHour: 12 }, { firstHour: 14, lastHour: 20 }]]]);
  const joined = stabilityCountryParts([x, y], spans, 'X')!;
  assert.deepEqual(joined.stability.parts[0].runs.map(run => run.valueRaw), ['10000', '20000', '30000']);
  assert.deepEqual(joined.stability.observed, [{ firstHour: 0, lastHour: 9 }, { firstHour: 10, lastHour: 12 }, { firstHour: 14, lastHour: 20 }]);
});
