import assert from 'node:assert/strict';
import test from 'node:test';
import { at, axisAt, drillAction, FOLLOW, pickedAt, recordedHour, regionDetail, regionFollows, regionHour, regionKey, regionWindow, resolveRegion,
  SETTINGS_DETAIL, SPEC, stageOf, type RegionAction, type TrackHour } from './regionTime.ts';
import { reduceDrill, CLOSED } from '../charts/drill/drillState.ts';
import type { BoundTrack, WidgetSpec, WidgetTime } from './model';

const track = (id: string, firstHour: number, lastHour: number, axisOffset: number) =>
  ({ id, axisOffset, branch: { id, firstHour, lastHour } }) as unknown as BoundTrack;
const late = track('late', 1000, 1009, 900);
const early = track('early', 0, 9, 0);
const tracks = [late, early];
const widget = (time: WidgetTime, trackId = 'late') => ({ id: 'w', kind: 'unknown-kind', trackIds: [trackId], countries: { kind: 'all' }, time }) as unknown as WidgetSpec;
const hour = (value: number) => value as TrackHour;

const SPECS: Record<string, WidgetTime> = {
  follow: { mode: 'follow' },
  campaign: { mode: 'campaign' },
  pin: { mode: 'pin', windows: { late: { firstOffset: 5, lastOffset: 5 } } },
  'pin with no window for its track': { mode: 'pin', windows: { other: { firstOffset: 5, lastOffset: 5 } } },
  range: { mode: 'range', firstOffset: 2, lastOffset: 6 },
  'range past the track': { mode: 'range', firstOffset: 2, lastOffset: 12 },
};
const ACTIONS: Record<string, RegionAction> = { spec: SPEC, follow: FOLLOW, 'picked 1003': at(hour(1003)), 'picked outside': pickedAt(late, 1012) };

test('every widget time, under every user action, on a track aligned 900 hours later: the region reads the hour that action means, as a track hour, or nothing', () => {
  const expected: Record<string, Record<string, (axis: number) => number | null>> = {
    follow: { spec: a => a + 900, follow: a => a + 900, 'picked 1003': () => 1003, 'picked outside': () => null },
    campaign: { spec: () => 1009, follow: a => a + 900, 'picked 1003': () => 1003, 'picked outside': () => null },
    pin: { spec: () => 1005, follow: a => a + 900, 'picked 1003': () => 1003, 'picked outside': () => null },
    'pin with no window for its track': { spec: () => null, follow: a => a + 900, 'picked 1003': () => 1003, 'picked outside': () => null },
    range: { spec: () => 1006, follow: a => a + 900, 'picked 1003': () => 1003, 'picked outside': () => null },
    'range past the track': { spec: () => null, follow: a => a + 900, 'picked 1003': () => 1003, 'picked outside': () => null },
  };
  for (const [specName, time] of Object.entries(SPECS)) for (const [actionName, action] of Object.entries(ACTIONS)) for (const axis of [99, 100, 104.6, 109, 110]) {
    const want = expected[specName][actionName](Math.floor(axis));
    const recorded = want !== null && want >= 1000 && want <= 1009 ? want : null;
    assert.equal(regionHour({ widget: widget(time), action }, late, tracks, axisAt(axis)), recorded,
      `${specName} × ${actionName} at axis hour ${axis}`);
  }
});

test('the region follows playback only when its own action, or its widget, follows; only the widget’s own fixed time asks for another time', () => {
  for (const [specName, time] of Object.entries(SPECS)) {
    assert.equal(regionFollows({ widget: widget(time), action: SPEC }), time.mode === 'follow', specName);
    assert.equal(regionFollows({ widget: widget(time), action: FOLLOW }), true, `${specName} told to follow`);
    assert.equal(regionFollows({ widget: widget(time), action: at(hour(1003)) }), false, `${specName} at a picked hour`);
    const own = { widget: widget(time), action: SPEC };
    assert.equal(regionDetail(own, null), time.mode === 'follow' ? undefined : SETTINGS_DETAIL, `${specName}: the remedy is the widget’s own setting`);
    assert.equal(regionDetail({ widget: widget(time), action: FOLLOW }, null), undefined, `${specName}: following playback outside is not a settings problem`);
    assert.equal(regionDetail({ widget: widget(time), action: pickedAt(late, 2000) }, null), undefined, `${specName}: a picked hour is not a settings problem`);
    assert.equal(regionDetail(own, hour(1005)), undefined, `${specName}: a recorded hour needs no remedy`);
  }
});

test('a pinned widget’s drill told to follow playback reads the playback hour, never the pin, and a picked hour never moves with the clock', () => {
  const pinned = widget(SPECS.pin);
  const opened = reduceDrill(CLOSED, { type: 'open', breakdown: 'detail', hour: 1005, playing: false, playhead: null });
  assert.deepEqual(drillAction(opened, late), at(hour(1005)));
  const followed = reduceDrill(opened, { type: 'follow' });
  assert.equal(regionHour({ widget: pinned, action: drillAction(followed, late) }, late, tracks, axisAt(108)), 1008);
  assert.equal(regionHour({ widget: pinned, action: drillAction(followed, late) }, late, tracks, axisAt(110)), null, 'past the track the drill is outside, not the pin');
  const picked = reduceDrill(followed, { type: 'pick', hour: 1003.7 });
  for (const axis of [100, 105, 110]) assert.equal(regionHour({ widget: widget(SPECS.follow), action: drillAction(picked, late) }, late, tracks, axisAt(axis)), 1003);
});

test('a region’s window ends at its hour; an unrecorded hour has no window', () => {
  const history = widget({ mode: 'follow' });
  assert.deepEqual(resolveRegion({ widget: history, action: FOLLOW }, late, tracks, axisAt(105)).window, { firstHour: 1005, lastHour: 1005 });
  assert.equal(resolveRegion({ widget: history, action: FOLLOW }, late, tracks, axisAt(111)).window, null);
  assert.equal(regionWindow({ widget: history, action: at(null) }, late, tracks, axisAt(105)), null);
  assert.equal(regionWindow({ widget: history, action: SPEC }, undefined, tracks, axisAt(105)), null, 'no track, no window');
});

test('a range is measured from its reference track, so a widget on two aligned tracks reads each at the same moment', () => {
  const both = { ...widget(SPECS.range), trackIds: ['late', 'early'] };
  assert.equal(regionHour({ widget: both, action: SPEC }, late, tracks, axisAt(0)), 1006);
  assert.equal(regionHour({ widget: both, action: SPEC }, early, tracks, axisAt(0)), null, 'the same moment on the early track is its hour 106, past its end');
  const aligned = [late, track('early', 100, 109, 0)];
  assert.equal(regionHour({ widget: both, action: SPEC }, aligned[1], aligned, axisAt(0)), 106, 'where the early track records that moment, it is read there');
});

test('only a recorded track hour is a track hour; the region key tells regions reading different times apart', () => {
  assert.equal(recordedHour(late, 999.9), null);
  assert.equal(recordedHour(late, 1009.9), 1009);
  assert.equal(recordedHour(late, Number.NaN), null);
  assert.equal(recordedHour(undefined, 1005), null);
  const keys = new Set(Object.values(ACTIONS).map(action => regionKey({ widget: widget(SPECS.pin), action })));
  assert.equal(keys.size, 4);
  assert.equal(regionKey({ widget: widget(SPECS.follow), action: SPEC }), regionKey({ widget: widget(SPECS.pin), action: FOLLOW }), 'following is one time whatever the spec');
});

test('the stages run in a fixed order: coverage, then the country, then membership, then the read', () => {
  assert.deepEqual(stageOf({ hour: null, detail: SETTINGS_DETAIL, country: 'absent', members: 'none', read: 'loading' }), { stage: 'outside', detail: SETTINGS_DETAIL });
  assert.deepEqual(stageOf({ hour: hour(1005), country: 'loading', members: 'none' }), { stage: 'loading' });
  assert.deepEqual(stageOf({ hour: hour(1005), country: 'absent', members: 'none' }), { stage: 'absent' });
  assert.deepEqual(stageOf({ hour: hour(1005), country: 'held', members: 'none', read: 'loading' }), { stage: 'no-members' });
  assert.deepEqual(stageOf({ hour: hour(1005), country: 'held', members: 'loading' }), { stage: 'loading' });
  assert.deepEqual(stageOf({ hour: hour(1005), country: 'held', members: 'held', read: 'loading' }), { stage: 'loading' });
  assert.deepEqual(stageOf({ hour: hour(1005), country: 'held', members: 'held', read: 'held' }), { stage: 'ready' });
});
