import test from 'node:test';
import assert from 'node:assert/strict';
import { registerWidgets } from '../../dashboard/definitions.ts';
import { addGridItemBelow, seekPositionFor } from '../../dashboard/model.ts';
import type { BoundTrack, GridLayouts, WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setOption, validOption } from '../../dashboard/options.ts';
import { jumpItem } from '../jump.ts';
import { breakdownOption, breakdownTitle, breakdownVariants, CLOSED, DRILL_OPTION, drillHour, FOLLOW_NEAR_HOURS, pinnedBreakdown, reduceDrill } from './drillState.ts';

test('a breakdown follows playback when playing or opened near the playhead, and pins a picked day', () => {
  const near = reduceDrill(CLOSED, { type: 'open', breakdown: 'income', hour: 1000, playing: false, playhead: 1000 + FOLLOW_NEAR_HOURS });
  assert.deepEqual(near, { breakdown: 'income', follow: true, hour: null });
  assert.equal(drillHour(near, 5000), 5000);
  const far = reduceDrill(CLOSED, { type: 'open', breakdown: 'income', hour: 1000.7, playing: false, playhead: 1000 + FOLLOW_NEAR_HOURS + 1 });
  assert.deepEqual(far, { breakdown: 'income', follow: false, hour: 1000 });
  assert.equal(reduceDrill(CLOSED, { type: 'open', breakdown: 'x', hour: 1, playing: true, playhead: 99_999 }).follow, true);
  // Whole-campaign views have no playhead, so the picked day holds.
  assert.equal(reduceDrill(CLOSED, { type: 'open', breakdown: 'x', hour: 1, playing: false, playhead: null }).follow, false);
  const picked = reduceDrill(near, { type: 'pick', hour: 2222 });
  assert.deepEqual(picked, { breakdown: 'income', follow: false, hour: 2222 });
  assert.equal(drillHour(picked, 9999), 2222);
  assert.deepEqual(reduceDrill(picked, { type: 'follow' }), near);
  assert.deepEqual(reduceDrill(picked, { type: 'close' }), CLOSED);
  assert.deepEqual(reduceDrill(CLOSED, { type: 'pick', hour: 5 }), CLOSED);
});

registerWidgets([{ kind: 'drill-test', version: 1, title: 'Drill test', icon: 'grid', timeModes: ['follow', 'campaign'], defaultTime: 'campaign',
  scope: 'history', data: true, history: true, minSize: { w: 3, h: 4 }, defaultSize: { w: 6, h: 8 } }]);

test('the drill settings are ordinary options, and a pinned breakdown shows it as the main view', () => {
  const show = breakdownOption([{ id: 'income', label: 'Daily income' }], 'Balance');
  assert.ok(validOption(DRILL_OPTION, 'popout'));
  assert.equal(resolveOptions([DRILL_OPTION, show], undefined).drill, 'inline');
  assert.deepEqual(setOption([show], undefined, 'show', 'income'), { show: 'income' });
  const source: WidgetSpec = { id: 'source', kind: 'drill-test', configVersion: 1, title: 'Political power', campaign: 'tracks', trackIds: ['t2'],
    countries: { kind: 'tags', tags: ['GER', 'ITA'] }, time: { mode: 'follow' }, contentScale: 125, options: { drill: 'popout' } };
  const pinned = pinnedBreakdown(source, { id: 'income', label: 'Daily income' }, 'main');
  assert.notEqual(pinned.id, source.id);
  assert.equal(pinned.title, 'Political power: Daily income');
  assert.deepEqual([pinned.campaign, pinned.trackIds, pinned.countries, pinned.time, pinned.contentScale], [source.campaign, source.trackIds, source.countries, source.time, 125]);
  assert.deepEqual(pinned.options, { drill: 'popout', show: 'income' });
});

test('a pinned widget goes directly below its source, as wide as it', () => {
  const layouts: GridLayouts = { lg: [{ i: 'source', x: 6, y: 4, w: 6, h: 8 }, { i: 'other', x: 0, y: 0, w: 6, h: 20 }] };
  const next = addGridItemBelow(layouts, { id: 'pin', kind: 'drill-test' } as WidgetSpec, 'source');
  assert.deepEqual(next.lg!.at(-1), { i: 'pin', x: 6, y: 12, w: 6, h: 8, minW: 3, minH: 4 });
  // Only the breakpoints the layout has: a breakpoint holding just the new widget is invalid.
  assert.deepEqual(Object.keys(next), ['lg']);
  assert.equal(addGridItemBelow({ xs: [] }, { id: 'pin', kind: 'drill-test' } as WidgetSpec, 'missing').xs!.at(-1)!.y, 0);
  // A widget already sitting right below moves down, so compaction cannot take the spot.
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

test('each breakdown is its own catalogue card, titled as Pin as widget titles it', () => {
  const show = breakdownOption([{ id: 'income', label: 'Daily income' }, { id: 'spending', label: 'Spending that day' }], 'Balance');
  const variants = breakdownVariants('Political power', [DRILL_OPTION, show]);
  assert.deepEqual(variants, [
    { key: 'show=income', initial: { title: 'Political power: Daily income', options: { show: 'income' } } },
    { key: 'show=spending', initial: { title: 'Political power: Spending that day', options: { show: 'spending' } } }]);
  const source: WidgetSpec = { id: 'source', kind: 'drill-test', configVersion: 1, title: 'Political power', campaign: 'default', trackIds: ['t'],
    countries: { kind: 'selected' }, time: { mode: 'campaign' }, contentScale: 100 };
  assert.equal(pinnedBreakdown(source, { id: 'income', label: 'Daily income' }, 't').title, variants[0].initial.title);
  assert.equal(breakdownTitle('Political power', 'Daily income'), 'Political power: Daily income');
  assert.deepEqual(breakdownVariants('Political power', [DRILL_OPTION]), [], 'a widget without a Show setting has no breakdown cards');
});
