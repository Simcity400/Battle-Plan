import test from 'node:test';
import assert from 'node:assert/strict';
import { BADGE_STRIP, badgesAt, visibleBadges } from './badges.ts';
import { plotFrame } from './frame.ts';
import { clipRuns } from '../dashboard/groups.ts';

const FRAME = plotFrame(400, 200, { left: 40, right: 10, top: BADGE_STRIP, bottom: 30 }, [1000, 2000], [0, 100], false);
const WAR = { hour: 1200, glyph: '⚔', label: 'War with FRA' };
const LAW = { hour: 1200, glyph: '⚙', label: 'Law: War Economy' };
const FOCUS = { hour: 1500, glyph: '★', label: 'Focus: Rhineland' };
const LATE = { hour: 1900, glyph: '★', label: 'Focus: Anschluss' };

test('badges show inside the view and never after the playhead', () => {
  assert.deepEqual(visibleBadges([WAR, FOCUS, LATE], [1000, 2000], null), [WAR, FOCUS, LATE]);
  assert.deepEqual(visibleBadges([WAR, FOCUS, LATE], [1000, 2000], 1600), [WAR, FOCUS]);
  assert.deepEqual(visibleBadges([WAR, FOCUS, LATE], [1300, 2000], null), [FOCUS, LATE]);
});

test('the pointer picks the nearest badge in the strip, with the other events of its hour', () => {
  const y = FRAME.plotTop - 10;
  assert.deepEqual(badgesAt({ x: FRAME.toX(1200) + 3, y }, FRAME, [WAR, LAW, FOCUS], null), [WAR, LAW]);
  assert.deepEqual(badgesAt({ x: FRAME.toX(1500) - 5, y }, FRAME, [WAR, LAW, FOCUS], null), [FOCUS]);
  assert.deepEqual(badgesAt({ x: FRAME.toX(1500) + 12, y }, FRAME, [FOCUS], null), []);
  assert.deepEqual(badgesAt({ x: FRAME.toX(1500), y: FRAME.plotTop + 40 }, FRAME, [FOCUS], null), []);
  assert.deepEqual(badgesAt({ x: FRAME.toX(1900), y }, FRAME, [LATE], 1600), []);
  const near = { hour: 1240, glyph: '★', label: 'Focus' };
  const between = (FRAME.toX(1200) + FRAME.toX(1240)) / 2;
  const hovered = badgesAt({ x: between, y }, FRAME, [WAR, near], null);
  assert.equal(new Set(hovered.map(badge => badge.hour)).size, 1);
});

test('triangle markers are found like glyph badges, in the strip above the plot', () => {
  const event = { hour: 1500, glyph: '▼', label: 'Declared war', color: '#123456', shape: 'triangle' as const };
  assert.deepEqual(badgesAt({ x: FRAME.toX(1500) + 2, y: FRAME.plotTop - 6 }, FRAME, [event, WAR], null), [event]);
  assert.deepEqual(badgesAt({ x: FRAME.toX(1500), y: FRAME.plotTop + 40 }, FRAME, [event], null), []);
  assert.deepEqual(visibleBadges([event], [1000, 2000], 1400), []);
});


test('a membership cut keeps a line break only where the run itself starts', () => {
  const runs = [{ firstHour: 100, lastHour: 110, value: 1, breakBefore: true }, { firstHour: 111, lastHour: 120, value: 2 }];
  const clipped = clipRuns(runs, [{ tag: 'GER', firstHour: 100, lastHour: 104 }, { tag: 'GER', firstHour: 106, lastHour: 115 }]);
  assert.deepEqual(clipped.map(run => [run.firstHour, run.lastHour, run.breakBefore ?? false]),
    [[100, 104, true], [106, 110, false], [111, 115, false]]);
});
