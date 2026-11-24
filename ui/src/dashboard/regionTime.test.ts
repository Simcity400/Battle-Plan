import assert from 'node:assert/strict';
import test from 'node:test';
import { OpenedSources } from './openedSources.ts';
import { REQUEST_HOURS } from '../generated/dashboard.ts';
import { at, axisAt, bucketEdges, bucketRead, checkedTime, coverageAt, coverageSpan, FOLLOW, GatedTime, refusedHours, validTime, OUTSIDE_MESSAGE, recordedHour, regionFollows, regionHour, regionKey, regionOutside, regionWindow, resolveRegion, SETTINGS_DETAIL, SPEC, UNRECORDED_MESSAGE, type RegionAction, type TrackHour } from './regionTime.ts';
import { drillAction, reduceDrill, CLOSED } from '../charts/drill/drillState.ts';
import { gateStage } from './regionGate.ts';
import { gatedRead, readData } from './gatedRead.ts';
import type { BoundTrack, WidgetSpec, WidgetTime } from './model';

const track = (id: string, firstHour: number, lastHour: number, axisOffset: number, gaps: { firstHour: number; lastHour: number; reason: string }[] = []) =>
  ({ id, axisOffset, branch: { id, firstHour, lastHour, gaps } }) as unknown as BoundTrack;
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
const ACTIONS: Record<string, RegionAction> = { spec: SPEC, follow: FOLLOW, 'picked 1003': at(hour(1003)), 'picked outside': at(1012) };

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
    const missing = regionOutside(own, late, tracks, axisAt(2000));
    assert.equal(missing === null, regionHour(own, late, tracks, axisAt(2000)) !== null, `${specName}: outside exactly when it has no recorded hour`);
    if (missing) assert.equal(!!missing.pinned, time.mode !== 'follow', `${specName}: the remedy is the widget’s own setting`);
    assert.equal(regionOutside({ widget: widget(time), action: FOLLOW }, late, tracks, axisAt(2000))?.pinned, undefined, `${specName}: following playback outside is not a settings problem`);
    assert.equal(regionOutside({ widget: widget(time), action: at(2000) }, late, tracks, axisAt(2000))?.pinned, undefined, `${specName}: a picked hour is not a settings problem`);
    assert.equal(regionOutside({ widget: widget(time), action: at(hour(1005)) }, late, tracks, axisAt(0)), null, `${specName}: a recorded hour needs no remedy`);
  }
});

test('a pinned widget’s drill told to follow playback reads the playback hour, never the pin, and a picked hour never moves with the clock', () => {
  const pinned = widget(SPECS.pin);
  const opened = reduceDrill(CLOSED, { type: 'open', breakdown: 'detail', hour: 1005, follow: false });
  assert.deepEqual(drillAction(opened), at(hour(1005)));
  const followed = reduceDrill(opened, { type: 'follow' });
  assert.equal(regionHour({ widget: pinned, action: drillAction(followed) }, late, tracks, axisAt(108)), 1008);
  assert.equal(regionHour({ widget: pinned, action: drillAction(followed) }, late, tracks, axisAt(110)), null, 'past the track the drill is outside, not the pin');
  const picked = reduceDrill(followed, { type: 'pick', hour: 1003.7 });
  for (const axis of [100, 105, 110]) assert.equal(regionHour({ widget: widget(SPECS.follow), action: drillAction(picked) }, late, tracks, axisAt(axis)), 1003);
});

test('a region’s window ends at its hour; an unrecorded hour has no window', () => {
  const history = widget({ mode: 'follow' });
  assert.deepEqual(resolveRegion({ widget: history, action: FOLLOW }, late, tracks, axisAt(105)).window, { firstHour: 1005, lastHour: 1005 });
  assert.equal(resolveRegion({ widget: history, action: FOLLOW }, late, tracks, axisAt(111)).window, null);
  assert.equal(regionWindow({ widget: history, action: at(Number.NaN) }, late, tracks, axisAt(105)), null);
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

test('a gated read reaches its data only when the gate is open and the read landed; every other state names itself', () => {
  const outside = { message: OUTSIDE_MESSAGE, detail: SETTINGS_DETAIL };
  const gate = (stage: Parameters<typeof gatedRead>[0]['stage'], at: number | null = 1005) => ({ stage, hour: at as never });
  const landed = { kind: 'dated', data: [1, 2], hour: 1005, retry() {} } as const;
  assert.equal(readData(gatedRead(gate({ stage: 'outside', outside }), landed)), undefined, 'data a gate refuses is never reachable, even if it landed');
  assert.equal(gatedRead(gate({ stage: 'outside', outside }), landed).state, 'outside');
  assert.equal(gatedRead(gate({ stage: 'open' }), landed, 'The range ends in a gap.').state, 'refused', 'a request the widget withheld says why, never pending');
  assert.equal(gatedRead(gate({ stage: 'open' }), { kind: 'dated', hour: 1005, retry() {} }).state, 'pending');
  assert.equal(gatedRead(gate({ stage: 'open' }), { kind: 'dated', hour: 1005, error: new Error('x'), retry() {} }).state, 'failed');
  assert.deepEqual(readData(gatedRead(gate({ stage: 'open' }), { kind: 'dated', data: [], hour: 1005, retry() {} })), [], 'a landed empty reply is ready with no rows');
  assert.equal(gatedRead(gate({ stage: 'open' }), { kind: 'dated', data: [1], hour: 1003, retry() {} }).state, 'pending', 'a reply landed for another hour never satisfies an exact pin');
  const followed = gatedRead(gate({ stage: 'open' }), { kind: 'dated', data: [1], hour: 1003, retry() {} }, null, true);
  assert.deepEqual(followed.state === 'ready' && followed.hour, 1003, 'ready carries the hour its reply landed for');
  assert.equal(gatedRead(gate({ stage: 'open' }), { kind: 'dated', data: [1], hour: null, retry() {} }, null, true).state, 'pending', 'a dated reply that names no landed hour is never ready');
  const branch = gatedRead(gate({ stage: 'open' }), { kind: 'branch', data: [1], retry() {} });
  assert.deepEqual(branch.state === 'ready' && branch.hour, null, 'a branch-wide history is ready with no hour of its own, never stamped with the gate hour');
});

test('the gate’s stages run in a fixed order, and a country that holds nothing once its history is known is absent, never loading', () => {
  const outside = { message: OUTSIDE_MESSAGE, detail: SETTINGS_DETAIL };
  const groups = { data: {}, error: undefined, retry() {} } as unknown as Parameters<typeof gateStage>[6];
  const country = (at: Partial<NonNullable<Parameters<typeof gateStage>[3]>>) => ({ chosen: 'GER', tag: 'GER', absent: false, reason: null, holders: null, historyHeld: true,
    aliases: null, ...at }) as unknown as Parameters<typeof gateStage>[3];
  const members = (selectionReason: string | null) => ({ selectionReason }) as unknown as Parameters<typeof gateStage>[7];
  const stage = (at: Partial<{ outside: typeof outside | null; hour: number | null; country: Parameters<typeof gateStage>[3]; members: Parameters<typeof gateStage>[7] }>) =>
    gateStage(late, at.outside ?? null, (at.hour === undefined ? 1005 : at.hour) as TrackHour | null, at.country ?? country({}), true, true, groups, at.members ?? members(null)).stage;
  assert.equal(stage({ outside, country: country({ absent: true }), members: members('none') }), 'outside', 'coverage comes first');
  assert.equal(stage({ hour: null }), 'loading');
  assert.equal(stage({ country: country({ tag: null, historyHeld: false }) }), 'loading', 'a country whose history is pending is loading');
  assert.equal(stage({ country: country({ tag: null }) }), 'absent', 'a country whose history is known but holds no tag here is absent');
  assert.equal(stage({ country: country({ aliases: new Map() }) }), 'absent', 'a held identity with no aliases at all is absent, not an empty open view');
  assert.equal(stage({ members: members('No members') }), 'no-members');
  assert.equal(stage({ members: { selectionReason: null, countries: new Set() } as unknown as Parameters<typeof gateStage>[7] }), 'no-members', 'a selection with no member at the hour is no-members, never an open empty read');
  assert.equal(stage({}), 'open');
});

test('a declared gap is not a recorded hour: every region time and pick inside it is unrecorded, with the recording’s own reason, and the hours around it are read', () => {
  const reason = 'Branch 1 block 1005 – 1006 is unreadable: checksum';
  const gapped = track('late', 1000, 1009, 900, [{ firstHour: 1005, lastHour: 1006, reason }]);
  for (const at of [1004, 1007]) assert.equal(recordedHour(gapped, at), at);
  const gap = { kind: 'not-recorded' as const, message: UNRECORDED_MESSAGE, detail: reason, span: { firstHour: 1005, lastHour: 1006 } };
  for (const at of [1005, 1006.5]) {
    assert.equal(recordedHour(gapped, at), null, `${at} is in the declared gap`);
    assert.deepEqual(coverageAt(gapped, at), { hour: null, outside: gap });
  }
  const pinned = { widget: widget({ mode: 'pin', windows: { late: { firstOffset: 5, lastOffset: 5 } } }), action: SPEC };
  assert.equal(regionHour(pinned, gapped, [gapped], axisAt(0)), null, 'a pin inside the gap has no hour, so nothing is chosen, saved or read there');
  assert.deepEqual(regionOutside(pinned, gapped, [gapped], axisAt(0)), { ...gap, pinned: true });
  assert.equal(regionHour({ widget: widget({ mode: 'follow' }), action: FOLLOW }, gapped, [gapped], axisAt(106)), null);
  const picked = { widget: widget({ mode: 'follow' }), action: at(1006.4) };
  assert.equal(regionHour(picked, gapped, [gapped], axisAt(0)), null, 'a pick inside the gap is no hour');
  assert.deepEqual(regionOutside(picked, gapped, [gapped], axisAt(0)), gap, 'and keeps the gap and its reason');
  assert.deepEqual(resolveRegion(picked, gapped, [gapped], axisAt(0)).outside, gap);
  const drilled = reduceDrill(reduceDrill(CLOSED, { type: 'open', breakdown: 'detail', hour: 1003, follow: false }), { type: 'pick', hour: 1005 });
  assert.deepEqual(regionOutside({ widget: widget({ mode: 'follow' }), action: drillAction(drilled) }, gapped, [gapped], axisAt(0)),
    gap, 'a drill picked inside the gap says why, never just “outside”');
  const overlapping = track('late', 0, 100, 0, [{ firstHour: 10, lastHour: 60, reason: 'a' }, { firstHour: 20, lastHour: 30, reason: 'b' }, { firstHour: 70, lastHour: 71, reason: 'c' }]);
  assert.equal(coverageAt(overlapping, 50).hour, null, 'a gap is found behind a later, shorter one');
  assert.equal(recordedHour(overlapping, 65), 65);
  const reasonOf = (hour: number) => { const covered = coverageAt(overlapping, hour); return covered.hour === null ? covered.outside.detail : 'recorded'; };
  for (const [hour, span] of [[15, [10, 19]], [25, [20, 30]], [45, [31, 60]], [65, [61, 69]]] as const) {
    assert.deepEqual(coverageSpan(overlapping, hour), span, `hour ${hour}: the reason changes at 20 and 31 inside the 10–60 gap`);
    for (let at = span[0]; at <= span[1]; at++) assert.equal(reasonOf(at), reasonOf(hour), `${at} shares ${hour}’s reason`);
  }
  const datedGap = track('dated', 43800000, 43800048, 0, [{ firstHour: 43800010, lastHour: 43800020, reason }]);
  const covered = coverageAt(datedGap, 43800015);
  assert.equal(covered.hour, null);
  assert.deepEqual(covered.outside, { kind: 'not-recorded', message: UNRECORDED_MESSAGE, detail: reason, span: { firstHour: 43800010, lastHour: 43800020 } },
    'calendar-dated gap keeps raw reason in detail and span separately without concatenating them');
});

const GAPPED = { firstHour: 100, lastHour: 109, gaps: [{ firstHour: 105, lastHour: 106, reason: 'unreadable' }] };

test('only checked hours leave for the native side: each field’s role comes from the contract, nested reads included, and requests widened past their declared type are refused by name', () => {
  assert.equal(refusedHours('series-state', { branchId: '1', reads: [{ country: 'GER', hour: 104 }] }, GAPPED), null);
  assert.match(refusedHours('series-state', { branchId: '1', reads: [{ country: 'GER', hour: 95 }] }, GAPPED) ?? '', /params\.reads\[0\]\.hour names hour 95/);
  assert.match(refusedHours('equipment-losses', { reads: [104, 105] }, GAPPED) ?? '', /hour 105, which was not recorded/);
  assert.equal(refusedHours('raid-log', { from: 105, to: 106 }, GAPPED), null, 'range bounds may lie in a gap');
  assert.match(refusedHours('series-state', { branchId: '1', family: { kind: 'supply', measure: 'state' }, reads: [{ tag: 'GER', hour: 104 }] }, GAPPED) ?? '',
    /params\.reads\[0\]\.tag is not a field the request type has/, 'a field the Rust request type does not name is refused before it is sent');
  assert.match(refusedHours('raid-log', { from: 99, to: 104 }, GAPPED) ?? '', /outside branch/);
  assert.match(refusedHours('equipment-produced', { rankHour: 105 }, GAPPED) ?? '', /rankHour names hour 105, which was not recorded/, 'a ranking hour is an exact read');
  assert.match(refusedHours('equipment-produced', { windowHour: 95 }, GAPPED) ?? '', /params\.windowHour is not a field the request type has/,
    'a name ending in Hour is not an hour unless the contract says so: a field the contract does not name is refused for its name, never read as an hour');
  assert.match(refusedHours('production-line-breakdown', { hour: 105 }, GAPPED) ?? '', /not recorded/, 'a detail read at one hour asks a recorded hour');
  assert.match(refusedHours('faction-lines', { from: 100, to: 109, members: [{ tag: 'GER', spans: [[90, 104]] }] }, GAPPED) ?? '', /members\[0\]\.spans\[0\]\[0\] names hour 90/);
  assert.match(refusedHours('series-tiles', { within: [[1, [{ firstHour: 100, lastHour: 120 }]]] }, GAPPED) ?? '', /within\[0\]\[1\]\[0\]\.lastHour names hour 120/);
  assert.match(refusedHours('series-tiles', { within: [[1, [[100, 120]]]] }, GAPPED) ?? '', /within\[0\]\[1\]\[0\] is not an object naming its fields/,
    'a positional span is refused, never passed unchecked');
  assert.match(refusedHours('naval-battle-details', ['1', [{ hour: 105 }]], GAPPED) ?? '', /params is not an object naming its fields/);
  assert.match(refusedHours('series-tiles', { family: { kind: 'player-apm', membership: [{ tag: 'GER', firstHour: 95, lastHour: 104 }] } }, GAPPED) ?? '',
    /family\.membership\[0\]\.firstHour names hour 95/, 'a nested family field is dated by the contract');
  assert.match(refusedHours('research-breakdown', { parts: [{ slot: 0, firstObservedHour: 105, lastHour: 106 }] }, GAPPED) ?? '',
    /parts\[0\]\.firstObservedHour names hour 105, which was not recorded/, 'an assignment’s first observation is an exact read');
  assert.match(refusedHours('no-such-product', {}, GAPPED) ?? '', /not a declared product/);
  const opened = new OpenedSources();
  opened.add({ id: 's', path: 's', revision: '1', metadata: { branches: [{ id: '1', ...GAPPED }] } } as never);
  assert.equal(opened.refusal({ sourceId: 's', product: 'follow', params: { id: 1, branchId: '1', clock: 0, query: { head: 'player-apm', params: { branchId: '1', firstHour: 100, lastHour: 109, windowHours: 720 } } } }), null);
  assert.match(opened.refusal({ sourceId: 's', product: 'follow', params: { id: 1, branchId: '1', clock: 0, query: { head: 'player-apm', params: { branchId: '1', firstHour: 90, lastHour: 109 } } } }) ?? '', /firstHour names hour 90/);
  assert.equal(opened.refusal({ sourceId: 's', product: 'follow', params: { id: 1, branchId: '1', clock: 0, query: { head: 'campaign-clock', params: { branchId: '2', firstHour: 100, lastHour: 109 } } } }),
    'follow names branch 2, not the followed branch 1', 'a followed head reads the subscription branch, never another');
  assert.equal(opened.refusal({ sourceId: 's', product: 'follow', params: { id: 1, branchId: '1', clock: 0, query: { head: 'campaign-clock', params: ['2', 100, 109] } } }),
    'follow: params is not an object naming its fields, as the contract declares', 'a positional head cannot hide its branch');
  const wide = { branchId: '1', hour: 95 };
  const narrow: { branchId: string } = wide;
  assert.match(opened.refusal({ sourceId: 's', product: 'country-fuel', params: narrow }) ?? '', /country-fuel: params\.hour names hour 95/);
  assert.equal(opened.refusal({ sourceId: 's', product: 'country-groups', params: { branchId: '1' } }), null);
  const follow = (hour?: number) => ({ id: 1, branchId: '1', clock: 0, query: { product: 'series-state', params: { branchId: '1', reads: [{ country: 'GER', ...hour === undefined ? {} : { hour } }] } } });
  assert.equal(opened.refusal({ sourceId: 's', product: 'follow', params: follow() }), null);
  assert.match(opened.refusal({ sourceId: 's', product: 'follow', params: follow(104) }) ?? '', /names no hour/, 'a followed read names no hour, even a recorded one');
  assert.equal(opened.refusal({ sourceId: 's', product: 'follow', params: { id: 1, branchId: '1', clock: 0, query: { product: 'series-state', params: { branchId: '1', reads: 'every' } } } }), null);
  assert.match(opened.refusal({ sourceId: 's', product: 'follow', params: { id: 1, branchId: '1', clock: 0, query: { panel: 'country-fuel', branchId: '1', hour: 104 } } }) ?? '', /names no hour/);
  assert.equal(opened.refusal({ sourceId: 's', product: 'follow-clock', params: { branchId: '1', hour: 95.5 } as never }), null, 'the clock envelope carries the playback position');
  assert.match(opened.refusal({ sourceId: 'closed', product: 'country-fuel', params: { branchId: '1', hour: 104 } as never }) ?? '', /not opened/);
});

const place = (segments: readonly string[], hour: number): unknown => {
  if (!segments.length) return hour;
  const [head, ...rest] = segments;
  const many = head.endsWith('[]'), key = many ? head.slice(0, -2) : head, inner = many ? [place(rest, hour)] : place(rest, hour);
  if (!/^\d+$/.test(key)) return { [key]: inner };
  const tuple: unknown[] = Array.from({ length: Number(key) }, () => null);
  tuple.push(inner);
  return tuple;
};

test('the validators follow every role the contract declares, for every product and field', () => {
  let checked = 0;
  for (const [product, fields] of Object.entries(REQUEST_HOURS)) for (const [path, role] of Object.entries(fields)) {
    const at = (hour: number) => place(path.split('.'), hour);
    const exact = role === 'read' || role === 'bucket';
    assert.equal(refusedHours(product, at(104), GAPPED), null, `${product} ${path}: a recorded hour`);
    assert.equal(refusedHours(product, at(105), GAPPED) === null, !exact, `${product} ${path} (${role}): an unrecorded hour`);
    assert.match(refusedHours(product, at(95), GAPPED) ?? '', /outside branch/, `${product} ${path}: an hour outside the branch`);
    assert.equal(refusedHours(product, at(104), GAPPED, true) === null, !exact, `${product} ${path} (${role}): followed`);
    checked++;
  }
  assert.ok(checked > 50, `the contract declares the dated fields (${checked})`);
});
test('a saved time is made only from checked windows, and persists nothing but its value', () => {
  const tracks = [{ id: 'main', branch: GAPPED }];
  assert.equal(validTime({ mode: 'pin', windows: { main: { firstOffset: 5, lastOffset: 5 } } }, tracks), null, 'a pin inside the gap');
  assert.equal(validTime({ mode: 'pin', windows: { main: { firstOffset: 3, lastOffset: 12 } } }, tracks), null, 'a pin past the track');
  assert.equal(validTime({ mode: 'pin', windows: { other: { firstOffset: 3, lastOffset: 3 } } }, tracks), null, 'a window for no track');
  assert.equal(validTime({ mode: 'range', firstOffset: 0, lastOffset: 6 }, tracks, 'main'), null, 'a range ending in the gap');
  const made = validTime({ mode: 'pin', windows: { main: { firstOffset: 3, lastOffset: 3 } } }, tracks);
  assert.ok(made instanceof GatedTime);
  assert.deepEqual(JSON.parse(JSON.stringify({ time: made })), { time: { mode: 'pin', windows: { main: { firstOffset: 3, lastOffset: 3 } } } });
  assert.ok(!({ ...made } instanceof GatedTime), 'a spread is not a saved time');
  assert.deepEqual(validTime({ mode: 'pin', windows: { main: { firstOffset: -164, lastOffset: 3 } } }, tracks)?.value, { mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 3 } } },
    'a week-long window pinned 3 hours into its branch is clipped to the branch, not refused');
  assert.deepEqual(validTime({ mode: 'range', firstOffset: -20, lastOffset: 3 }, tracks, 'main')?.value, { mode: 'range', firstOffset: 0, lastOffset: 3 });
  const short = { id: 'short', branch: { firstHour: 100, lastHour: 101, gaps: [] } };
  assert.deepEqual(validTime({ mode: 'pin', windows: { main: { firstOffset: 3, lastOffset: 3 }, short: { firstOffset: 3, lastOffset: 3 } } }, [...tracks, short])?.value,
    { mode: 'pin', windows: { main: { firstOffset: 3, lastOffset: 3 } } }, 'a short track does not invalidate the pin of the others');
  assert.match(checkedTime({ mode: 'pin', windows: { main: { firstOffset: 5, lastOffset: 5 } } }, tracks).refused ?? '', /nothing to pin/, 'a refused pin says why');
});

test('a product’s buckets are boundaries, and each bucket is read at one recorded hour', () => {
  const branch = { branch: { firstHour: 100, lastHour: 2000, gaps: [{ firstHour: 720, lastHour: 730, reason: 'unreadable' }] } };
  assert.deepEqual(bucketEdges(branch, 720), [720, 1440]);
  assert.equal(bucketRead(branch, 105, 720), 100);
  assert.equal(bucketRead(branch, 719, 720), 100, 'every hour of a bucket asks the same representative');
  assert.equal(bucketRead(branch, 1000, 720), 731, 'a bucket opening in a gap is read at its first recorded hour');
  assert.equal(bucketRead(branch, null, 720), null);
  assert.equal(bucketRead(branch, 1000, 720, [{ firstHour: 900, lastHour: 1300 }]), 900, 'a held country’s bucket is read at its first recorded hour inside the country’s runs');
  assert.equal(bucketRead(branch, 1000, 720, [{ firstHour: 100, lastHour: 719 }]), null, 'a bucket outside the country’s runs has no representative');
});

test('coverage spans name the run of hours that share one coverage answer', () => {
  const track = { branch: GAPPED };
  const answer = (hour: number) => { const covered = coverageAt(track, hour); return covered.hour === null ? covered.outside : 'recorded'; };
  for (const [hour, span] of [[95, [-Infinity, 99]], [100, [100, 104]], [104, [100, 104]], [105, [105, 106]], [107, [107, 109]], [110, [110, Infinity]]] as const) {
    assert.deepEqual(coverageSpan(track, hour), span, `hour ${hour}`);
    for (let at = Math.max(span[0], 90); at <= Math.min(span[1], 115); at++) assert.deepEqual(answer(at), answer(hour), `${at} shares ${hour}’s answer`);
  }
});
