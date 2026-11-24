import test from 'node:test';
import assert from 'node:assert/strict';
import { CLOSED, drillAction, reduceDrill } from '../../charts/drill/drillState.ts';
import type { DrillState } from '../../charts/drill/drillState';
import type { AgencyUpgradesSelection } from '../../dashboard/gameWindows';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { axisAt, regionHour } from '../../dashboard/regionTime.ts';
import { gameWindowSelectionAt, openGameWindow } from './gameWindowOpen.ts';

const TRACK = { id: 'wide', axisOffset: 0, branch: { id: 'wide', firstHour: 0, lastHour: 100_000, gaps: [] } } as unknown as BoundTrack;
const FOLLOWING = { trackIds: ['wide'], time: { mode: 'follow' } } as unknown as WidgetSpec;
const windowHour = (state: DrillState, playhead: number) =>
  regionHour({ widget: FOLLOWING, action: drillAction(state) }, TRACK, [TRACK], axisAt(playhead));

test('a window opened from a row during playback keeps the clicked subject and hour until the user follows', () => {
  const clicked: AgencyUpgradesSelection = {
    kind: 'agencyUpgrades', sourceId: 'src', revision: 'rev', branchId: 'wide', hour: 1000, tag: 'GER',
  };
  const { opened, drill } = openGameWindow('scope', 'agency:7', clicked, 'window');
  let state = reduceDrill(CLOSED, drill);

  const later = windowHour(state, 1240);
  assert.equal(later, 1000, 'the clock moved on; the window stays at the hour of the click');
  assert.deepEqual(gameWindowSelectionAt(opened, later ?? Number.NaN), clicked);
  assert.equal(opened.key, 'agency:7');

  state = reduceDrill(state, { type: 'follow' });
  const followed = windowHour(state, 1240);
  assert.equal(followed, 1240, 'Follow is the user’s choice, and then the window moves with the clock');
  assert.deepEqual(gameWindowSelectionAt(opened, followed ?? Number.NaN), { ...clicked, hour: 1240 },
    'following moves the hour only: the subject is still the clicked agency');

  state = reduceDrill(state, { type: 'pick', hour: 1300 });
  assert.equal(windowHour(state, 2000), 1300);
});
