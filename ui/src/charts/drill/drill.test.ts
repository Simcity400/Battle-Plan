import test from 'node:test';
import assert from 'node:assert/strict';
import { registerWidgets } from '../../dashboard/definitions.ts';
import { addGridItemBelow, seekPositionFor } from '../../dashboard/model.ts';
import type { BoundTrack, GridLayouts, WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setOption } from '../../dashboard/options.ts';
import { parseTemplate } from '../../dashboard/storage.ts';
import { OPTIONS as MINEFIELD_OPTIONS } from '../../widgets/minefields/definition.ts';
import { partsRaceOptions } from '../race/partsRaceOptions.ts';
import { jumpItem } from '../jump.ts';
import { breakdownOption, CLOSED, drillAction, FOLLOW_NEAR_HOURS, opensFollowing, reduceDrill, savedDrill } from './drillState.ts';
import type { DrillState } from './drillState';
import { axisAt, regionHour, SPEC } from '../../dashboard/regionTime.ts';

const WIDE = { id: 'wide', axisOffset: 0, branch: { id: 'wide', firstHour: 0, lastHour: 100_000, gaps: [] } } as unknown as BoundTrack;
const widget = (time: WidgetSpec['time']) => ({ trackIds: ['wide'], time }) as unknown as WidgetSpec;
const drillAt = (state: DrillState, playhead: number, time: WidgetSpec['time'] = { mode: 'follow' }) =>
  regionHour({ widget: widget(time), action: drillAction(state) }, WIDE, [WIDE], axisAt(playhead));

test('one reducer for every drill: open, pick, follow and close, with a saved breakdown reading its widget time until the user acts', () => {
  const followed = reduceDrill(CLOSED, { type: 'open', breakdown: 'income', hour: 1000, follow: true });
  assert.deepEqual(followed, { breakdown: 'income', spec: false, follow: true, hour: null });
  assert.equal(drillAt(followed, 5000), 5000);
  const picked = reduceDrill(followed, { type: 'pick', hour: 2222.7 });
  assert.deepEqual(picked, { breakdown: 'income', spec: false, follow: false, hour: 2222 });
  assert.equal(drillAt(picked, 9999), 2222);
  assert.deepEqual(reduceDrill(picked, { type: 'follow' }), followed);
  assert.deepEqual(reduceDrill(picked, { type: 'close' }), CLOSED);
  assert.deepEqual(reduceDrill(CLOSED, { type: 'pick', hour: 5 }), CLOSED);
  assert.deepEqual(reduceDrill(CLOSED, { type: 'follow' }), CLOSED);
  const pin = { mode: 'pin', windows: { wide: { firstOffset: 1005, lastOffset: 1005 } } } as const;
  const saved = savedDrill('fuel');
  assert.equal(drillAt(saved, 1010, pin), 1005, 'a saved breakdown reads its widget’s pin');
  const told = reduceDrill(saved, { type: 'follow' });
  assert.equal(drillAt(told, 1010, pin), 1010, 'Follow on a saved breakdown follows playback, not the pin');
  assert.equal(drillAt(told, 1005, pin), 1005, 'and seeking back is inside the recording at the playback hour');
  assert.equal(drillAt(reduceDrill(told, { type: 'pick', hour: 1002 }), 1010, pin), 1002);
  const attached = reduceDrill(CLOSED, { type: 'attach', breakdown: 'row' });
  assert.equal(drillAt(attached, 1010, pin), 1005, 'an attached item drill reads its widget’s own time');
});

test('a drill opens following playback only under a following widget, when playing or opened near the playhead', () => {
  const follows = { widget: widget({ mode: 'follow' }), action: SPEC }, pinned = { widget: widget({ mode: 'pin', windows: {} }), action: SPEC };
  assert.equal(opensFollowing(follows, 1000, false, 1000 + FOLLOW_NEAR_HOURS), true);
  assert.equal(opensFollowing(follows, 1000, false, 1000 + FOLLOW_NEAR_HOURS + 1), false);
  assert.equal(opensFollowing(follows, 1, true, 99_999), true);
  assert.equal(opensFollowing(pinned, 1000, true, 1000), false, 'a pinned widget’s drill holds the drawn hour, even while playing');
});

registerWidgets([{ kind: 'drill-test', version: 1, title: 'Drill test', icon: 'grid', timeModes: ['follow', 'campaign'], defaultTime: 'campaign',
  scope: 'history', data: true, history: true, minSize: { w: 3, h: 4 }, defaultSize: { w: 6, h: 8 } }]);

test('the drill settings are ordinary options and a saved dashboard holding a removed drill key loads cleanly', () => {
  const show = breakdownOption([{ id: 'income', label: 'Daily income' }], 'Balance');
  assert.deepEqual(setOption([show], undefined, 'show', 'income'), { show: 'income' });

  const saved = parseTemplate({
    version: 1,
    name: 'Saved layout',
    alignment: 'calendar',
    roles: [{ id: 'primary', label: 'Primary recording', selectedTag: null }],
    tracks: [{ id: 'main', roleId: 'primary', label: 'Main campaign', branchIndex: 0, anchorOffset: null }],
    widgets: [{
      id: 'w1', kind: 'minefields', configVersion: 1, title: 'Minefields', trackIds: ['main'],
      countries: { kind: 'all' }, time: { mode: 'follow' }, options: { drill: 'popout', order: 'name' },
    }],
    layouts: { lg: [{ i: 'w1', x: 0, y: 0, w: 6, h: 8 }] },
  });
  assert.ok(saved, 'a saved dashboard with legacy { drill: "popout" } parses cleanly');
  const stored = saved.widgets[0]!.options;
  assert.deepEqual(resolveOptions(MINEFIELD_OPTIONS, stored), { regions: '', order: 'name' });
  assert.equal('drill' in resolveOptions(MINEFIELD_OPTIONS, stored), false);
  assert.equal('drill' in resolveOptions(partsRaceOptions('type'), { drill: 'popout' }), false);
});

test('a pinned widget goes directly below its source, as wide as it', () => {
  const layouts: GridLayouts = { lg: [{ i: 'source', x: 6, y: 4, w: 6, h: 8 }, { i: 'other', x: 0, y: 0, w: 6, h: 20 }] };
  const next = addGridItemBelow(layouts, { id: 'pin', kind: 'drill-test' } as WidgetSpec, 'source');
  assert.deepEqual(next.lg!.at(-1), { i: 'pin', x: 6, y: 12, w: 6, h: 8, minW: 3, minH: 4 });
  assert.deepEqual(Object.keys(next), ['lg']);
  assert.equal(addGridItemBelow({ xs: [] }, { id: 'pin', kind: 'drill-test' } as WidgetSpec, 'missing').xs!.at(-1)!.y, 0);
  const crowded: GridLayouts = { lg: [{ i: 'source', x: 6, y: 4, w: 6, h: 8 }, { i: 'under', x: 0, y: 12, w: 12, h: 5 }] };
  const placed = addGridItemBelow(crowded, { id: 'pin', kind: 'drill-test' } as WidgetSpec, 'source').lg!;
  assert.equal(placed.find(item => item.i === 'under')!.y, 20);
  assert.equal(placed.find(item => item.i === 'pin')!.y, 12);
});

const TRACK = { id: 't1', axisOffset: 500, branch: { firstHour: 1000, lastHour: 2000 } } as unknown as BoundTrack;

test('jump to asks the clock for a position inside the branch only', () => {
  assert.equal(seekPositionFor(TRACK, 1500.9), 1000);
  assert.equal(seekPositionFor(TRACK, 999), null);
  assert.equal(seekPositionFor(TRACK, 2001), null);
  const asked: [string, number][] = [];
  const runtime = { tracks: [TRACK] as never, requestSeek: (trackId: string, hour: number) => { asked.push([trackId, hour]); return true; } };
  const item = jumpItem(runtime, 't1', 1500);
  assert.equal(item.disabled, false);
  assert.match(item.label, /^Jump to /);
  item.run();
  assert.deepEqual(asked, [['t1', 1500]]);
  assert.equal(jumpItem(runtime, 't1', 5000).disabled, true);
  assert.equal(jumpItem(runtime, 'missing', 1500).disabled, true);
});

