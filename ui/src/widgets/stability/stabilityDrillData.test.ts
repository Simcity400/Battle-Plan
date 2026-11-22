import { gatedTime } from '../../test-support/gated.ts';
import { persistedSpec } from '../../dashboard/regionTime.ts';
import test from 'node:test';
import type { DrillState } from '../../charts/drill/drillState';
import assert from 'node:assert/strict';
import { plotFrame } from '../../charts/frame.ts';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState.ts';
import { axisAt, regionHour } from '../../dashboard/regionTime.ts';
import { drillAction } from '../../charts/drill/drillState.ts';
import { createWidget, resolveWindow } from '../../dashboard/model.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import '../../widgets/definitions.ts';
import { chooseStabilityActor, pickStabilityLine, pinStabilityDrill, stabilityPickAction } from './stabilityDrillData.ts';
import type { StabilityLine } from './stabilityDrillData';

const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 100], false);
const line = (tag: string, measure: StabilityLine['measure'], value: number): StabilityLine => ({ tag, measure,
  runs: [{ firstHour: 0, lastHour: 100, value }] });
const pointer = (value: number, hour = 49) => ({ frame, until: null, point: { x: frame.toX(hour), y: frame.toY(value) } });
const WIDE = { id: 'wide', axisOffset: 0, branch: { id: 'wide', firstHour: 0, lastHour: 100_000 } } as unknown as BoundTrack;
const drillAt = (state: DrillState, playhead: number) =>
  regionHour({ widget: { trackIds: ['wide'], time: { mode: 'follow' } } as unknown as WidgetSpec, action: drillAction(state) }, WIDE, [WIDE], axisAt(playhead));

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
    let drill = reduceDrill(CLOSED, stabilityPickAction(picked, false, () => true));
    assert.equal(drill.follow, false);
    assert.equal(drillAt(drill, 100), 49);
    assert.equal(chooseStabilityActor(picked, 'missing'), picked);
    const chosen = chooseStabilityActor(picked, 'b');
    assert.equal(chosen.measure, 'warSupport');
    assert.equal(chosen.tag, 'b');
    drill = reduceDrill(drill, stabilityPickAction(chosen, true, () => true));
    assert.equal(drillAt(drill, 100), 49);
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
  for (const follows of [false]) {
    const drill = reduceDrill(CLOSED, stabilityPickAction(pick, false, () => follows));
    assert.equal(drill.follow, false);
    assert.equal(drillAt(drill, 100), 20);
    const following = reduceDrill(drill, { type: 'follow' });
    assert.equal(drillAt(following, 80), 80, 'explicit Follow reads the actual aligned source clock');
  }
  assert.equal(reduceDrill(CLOSED, stabilityPickAction(pick, false, () => true)).follow, true);
});

test('Pin preserves picked actor, measure and branch-relative hour; explicit Follow makes a Follow widget', () => {
  const widget = createWidget('stability', 'track', 'stability-test');
  widget.countries = { kind: 'tags', tags: ['a', 'b'] };
  const pin = persistedSpec(pinStabilityDrill(widget, { id: 'warSupport', label: 'War support parts' }, 'track', 'b', gatedTime({ mode: 'pin', windows: { track: { firstOffset: 20, lastOffset: 20 } } })));
  assert.deepEqual(pin.countries, { kind: 'tags', tags: ['b'] });
  assert.deepEqual(pin.time, { mode: 'pin', windows: { track: { firstOffset: 20, lastOffset: 20 } } });
  assert.equal(pin.options?.show, 'warSupport');
  const track = { id: 'track', axisOffset: 900, branch: { firstHour: 1000, lastHour: 2000 } } as BoundTrack;
  assert.deepEqual(resolveWindow(pin, track, [track], 500), { firstHour: 1020, lastHour: 1020 });
  const live = persistedSpec(pinStabilityDrill(widget, { id: 'stability', label: 'Stability parts' }, 'track', 'b', gatedTime({ mode: 'follow' })));
  assert.equal(live.time.mode, 'follow');
  assert.equal(resolveWindow(live, track, [track], 200)!.lastHour, 1100);
});

