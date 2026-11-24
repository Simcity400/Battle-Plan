import test from 'node:test';
import type { DrillState } from '../../charts/drill/drillState';
import assert from 'node:assert/strict';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState.ts';
import { axisAt, regionHour } from '../../dashboard/regionTime.ts';
import { drillAction } from '../../charts/drill/drillState.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import '../../widgets/definitions.ts';
import { chooseStabilityActor, stabilityPickAction } from './stabilityDrillData.ts';
import type { StabilityPick } from './stabilityDrillData';

const WIDE = { id: 'wide', axisOffset: 0, branch: { id: 'wide', firstHour: 0, lastHour: 100_000 } } as unknown as BoundTrack;
const drillAt = (state: DrillState, playhead: number) =>
  regionHour({ widget: { trackIds: ['wide'], time: { mode: 'follow' } } as unknown as WidgetSpec, action: drillAction(state) }, WIDE, [WIDE], axisAt(playhead));

test('overlap and keyboard choices retain actor-specific measure and held hour while playback advances', () => {
  const picked: StabilityPick = { hour: 49, candidates: ['a', 'b'], tag: null, measure: 'stability',
    lines: [{ tag: 'a', measure: 'stability' }, { tag: 'b', measure: 'warSupport' }] };
  {
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

test('Campaign, Pin and Range opens hold the drawn hour even while the global clock is playing', () => {
  const pick: StabilityPick = { hour: 20, candidates: ['actor'], lines: [{ tag: 'actor', measure: 'warSupport' }], tag: 'actor', measure: 'warSupport' };
  for (const follows of [false]) {
    const drill = reduceDrill(CLOSED, stabilityPickAction(pick, false, () => follows));
    assert.equal(drill.follow, false);
    assert.equal(drillAt(drill, 100), 20);
    const following = reduceDrill(drill, { type: 'follow' });
    assert.equal(drillAt(following, 80), 80, 'explicit Follow reads the actual aligned source clock');
  }
  assert.equal(reduceDrill(CLOSED, stabilityPickAction(pick, false, () => true)).follow, true);
});
