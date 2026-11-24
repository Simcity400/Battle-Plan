import assert from 'node:assert/strict';
import test from 'node:test';
import type { ResearchHistory, ResearchSlotHistory } from './types';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { researchAt, researchFolderList, researchFolderLooks } from './researchData.ts';
import { completedTooltip, coverageAt, recordedResearchLanes, researchDateRange, researchItemDate, researchLanes, researchLanesAt, researchRows } from './researchLayout.ts';
import type { ResearchEntry, ResearchSlotState } from '../../generated/dashboard';

const base = HOUR_EPOCH + 1944 * 8760 + 12;
const day = (n: number) => base + n * 24;
const entry = (token: string, first: number, end: number | null, outcome: ResearchEntry['outcome'] = 'completed', startedAfter: number | null = first - 24): ResearchEntry => ({
  token, technologyHandle: token.charCodeAt(0), databaseIndex: token.charCodeAt(0), continuityStartHour: day(0), level: 0, firstObservedHour: first, startedAfterHour: startedAfter,
  lastObservedHour: end === null ? first : end - 24, endedByHour: end, outcomeObservedHour: null, outcome,
});
const slot = (index: number, availability: { firstHour: number; lastHour: number }[], entries: ResearchEntry[], states: ResearchSlotState[] = []): ResearchSlotHistory => ({
  index, availability, entries, states, progress: [],
});
const read = (token: string, firstHour: number, lastHour: number, entryFirstObservedHour: number): ResearchSlotState => ({ firstHour, lastHour, token, databaseIndex: token.charCodeAt(0),
  entryFirstObservedHour, speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0', aheadReductionYearsRaw: '0', details: null });
const lanesAt = (history: ResearchHistory, hour: number) => researchLanes(researchAt(history, hour), coverageAt(history.coverage, hour), hour);
const kinds = (history: ResearchHistory, hour: number) => lanesAt(history, hour).map(lane => lane.items.map(item => item.kind));

test('indexed lanes match clipped histories through gaps, late confirmations and slot changes in either playback direction', () => {
  const history: ResearchHistory = { coverage: [{ firstHour: 0, lastHour: 40 }, { firstHour: 60, lastHour: 100 }], slots: [
    slot(0, [{ firstHour: 0, lastHour: 40 }, { firstHour: 60, lastHour: 100 }], [
      { ...entry('A', 0, 10), outcomeObservedHour: 30 }, entry('B', 10, 25, 'switched'), entry('C', 25, 35),
      entry('D', 60, 70), entry('E', 70, null, 'active'),
    ]),
    slot(1, [{ firstHour: 20, lastHour: 40 }, { firstHour: 60, lastHour: 80 }], [entry('G', 20, 30), entry('H', 30, 38, 'switched')]),
    slot(2, [{ firstHour: 60, lastHour: 100 }], [entry('F', 65, 80)]),
  ] };
  const project = (lanes: ReturnType<typeof researchLanes>) => lanes.map(lane => ({ index: lane.slot.index, appeared: lane.appeared,
    offset: lane.offset, items: lane.items.map(item => { const { slot: _slot, ...value } = item as typeof item & { slot?: ResearchSlotHistory }; return value; }) }));
  const presented = researchLanesAt(history, 80);
  const rewound = recordedResearchLanes(history, 19);
  assert.notDeepEqual(project(presented), project(rewound));
  assert.deepEqual(rewound.map(l => l.items.map(item => item.kind)), [[]]);
  const hours = Array.from({ length: 104 }, (_, at) => at - 1);
  for (const hour of [...hours, ...[...hours].reverse()]) assert.deepEqual(project(researchLanesAt(history, hour)), project(lanesAt(history, hour)), `hour ${hour}`);
  assert.deepEqual(researchLanesAt(history, NaN), []);
});

test('5000 indexed completion rows retain their identities between observed changes', () => {
  const history: ResearchHistory = { coverage: [{ firstHour: 0, lastHour: 20000 }], slots: [slot(0, [{ firstHour: 0, lastHour: 20000 }],
    Array.from({ length: 5000 }, (_, at) => ({ ...entry(`tech${at}`, at * 3, at * 3 + 1), outcomeObservedHour: at * 3 + 2 })))] };
  const before = researchLanesAt(history, 15001), items = before[0].items;
  assert.equal(items.length, 5000);
  assert.equal(researchLanesAt(history, 15002), before);
  assert.equal(researchLanesAt(history, 15002)[0].items, items);
  assert.equal(researchLanesAt(history, 1)[0].items.length, 0, 'a later completion confirmation stays hidden');
  assert.equal(researchLanesAt(history, 2)[0].items.length, 1);
  assert.equal(researchLanesAt(history, 15002)[0].items[0], items[0], 'rewind preserves each indexed row');
});

test('each slot lists only its completions, packed, and a slot that opened later starts at the row the others had reached', () => {
  const history: ResearchHistory = { coverage: [{ firstHour: day(0), lastHour: day(200) }], slots: [
    slot(0, [{ firstHour: day(0), lastHour: day(200) }], [entry('A', day(0), day(50)), entry('B', day(50), day(150), 'switched'), entry('C', day(150), null, 'active')]),
    slot(1, [{ firstHour: day(0), lastHour: day(200) }], [entry('D', day(0), day(120)), entry('E', day(120), day(160)), entry('F', day(160), null, 'active')]),
    slot(2, [{ firstHour: day(130), lastHour: day(200) }], [entry('G', day(130), day(145)), entry('H', day(145), null, 'active')]),
  ] };
  const lanes = lanesAt(history, day(200));
  assert.deepEqual(lanes.map(lane => lane.offset), [0, 0, 1], 'slot 3 starts below the most any slot had finished when it opened');
  assert.deepEqual(kinds(history, day(200)), [['completed'], ['completed', 'completed'], ['opened', 'completed']]);
  assert.equal(researchItemDate(lanes[2].items[0] as Extract<typeof lanes[2]['items'][number], { kind: 'opened' }>), 'Opened by 11 May 1944');
  assert.deepEqual(lanesAt(history, day(100)).map(lane => lane.slot.index), [0, 1]);
});

test('a recording gap appears only in the slots whose sequence spans it, and closing and late first sightings are marked', () => {
  const history: ResearchHistory = { coverage: [{ firstHour: day(0), lastHour: day(99) }, { firstHour: day(120), lastHour: day(200) }], slots: [
    slot(0, [{ firstHour: day(0), lastHour: day(99) }, { firstHour: day(120), lastHour: day(200) }], [entry('X', day(0), day(40)), entry('Y', day(120), day(160), 'completed', null)]),
    slot(1, [{ firstHour: day(0), lastHour: day(60) }], [entry('Z', day(0), day(30))]),
    slot(2, [{ firstHour: day(120), lastHour: day(200) }], []),
  ] };
  const lanes = lanesAt(history, day(200));
  assert.deepEqual(lanes.map(lane => lane.items.map(item => item.kind)), [['completed', 'gap', 'completed'], ['completed', 'closed'], ['opened']]);
  const gap = lanes[0].items[1];
  assert.equal(gap.kind === 'gap' && researchDateRange(gap.hour, gap.lastHour), '10 Apr – 1 May 1944');
  const opened = lanes[2].items[0];
  assert.equal(opened.kind === 'opened' && opened.known, false, 'first listed after a gap: when it opened is not recorded');
  assert.equal(opened.kind === 'opened' && researchItemDate(opened), 'Recorded from 1 May 1944');
  const closed = lanes[1].items[1];
  assert.equal(closed.kind === 'closed' && researchItemDate(closed), 'Closed by 2 Mar 1944');
  assert.deepEqual(researchRows(lanes, researchAt(history, day(200)), coverageAt(history.coverage, day(200))).map(row => row.kind),
    ['completed', 'completed', 'gap', 'completed']);
});

function movedHistory(progress: string): ResearchHistory {
  const opening = read('T', day(10), day(40) - 1, day(10));
  return { coverage: [{ firstHour: day(0), lastHour: day(200) }], slots: [
    { ...slot(0, [{ firstHour: day(0), lastHour: day(200) }], [entry('U', day(0), day(5), 'completed', null), entry('T', day(10), day(40), 'switched')], [opening]),
      progress: [{ firstHour: day(10), lastHour: day(11) - 1, entryFirstObservedHour: day(10), baseDaysRaw: '10000000', progressRaw: progress }] },
    slot(1, [{ firstHour: day(0), lastHour: day(200) }], [entry('T', day(40), day(90))]),
  ] };
}
const completedRows = (history: ResearchHistory) => researchRows(lanesAt(history, day(200)), researchAt(history, day(200)), history.coverage);
const names = { name: (token: string) => `Tech ${token}`, icon: () => undefined, folder: () => undefined };

test('a completion takes its start and time from its first assignment, including earlier slots', () => {
  const rows = completedRows(movedHistory('0'));
  const t = rows.find(row => row.kind === 'completed' && row.token === 'T');
  const u = rows.find(row => row.kind === 'completed' && row.token === 'U');
  assert.ok(t?.kind === 'completed' && u?.kind === 'completed');
  assert.deepEqual([t.started, t.startSlot, t.tookHours], [{ hour: day(10), known: true }, 0, day(90) - day(10)]);
  assert.deepEqual([u.started, u.startSlot, u.tookHours], [{ hour: day(0), known: false }, null, null], 'a start before the first read is unknown');
  assert.deepEqual(completedTooltip(t, names).rows.map(row => [row.label, row.value]), [
    ['Completed', 'by 1 Apr 1944 · 13:00'], ['Still researching on', '31 Mar 1944 · 13:00'], ['Started', '11 Jan 1944'],
    ['Took', '80 days'], ['Slot', '2 · started in slot 1'],
  ]);
});

test('resumed research with earlier progress claims no start and no time taken', () => {
  const t = completedRows(movedHistory('3000000')).find(row => row.kind === 'completed' && row.token === 'T');
  assert.ok(t?.kind === 'completed');
  assert.deepEqual([t.started, t.startSlot, t.tookHours], [{ hour: day(10), known: false }, 0, null]);
  assert.deepEqual(completedTooltip(t, names).rows.map(row => [row.label, row.value]), [
    ['Completed', 'by 1 Apr 1944 · 13:00'], ['Still researching on', '31 Mar 1944 · 13:00'], ['Started', 'by 11 Jan 1944'],
    ['Slot', '2 · started in slot 1'],
  ]);
});

test('folder colours follow the content order of the folders a history shows, generating distinct colours past the palette', () => {
  const reply = { folders: [{ key: 'inf', name: 'Infantry' }, { key: 'air', name: 'Air' }, { key: 'nav', name: null }, { key: 'unused', name: 'Unused' }],
    technologyFolders: [{ token: 't3', folder: 'nav' }, { token: 't1', folder: 'air' }, { token: 't2', folder: 'inf' }, { token: 't4', folder: null }] };
  const looks = researchFolderLooks(reply, ['t3', 't1', 't2', 't4'], ['red', 'blue']);
  const list = researchFolderList(looks);
  assert.deepEqual(list.slice(0, 2).map(look => [look.key, look.name, look.colour]),
    [['inf', 'Infantry', 'red'], ['air', 'Air', 'blue']]);
  assert.equal(list[2].key, 'nav');
  assert.equal(list[2].name, 'Folder name unavailable');
  assert.match(list[2].colour, /^#[0-9a-f]{6}$/);
  assert.equal(looks.get('t3')?.colour, list[2].colour);
  assert.equal(looks.has('t4'), false, 'a technology in no shown folder has no look');
  assert.equal(researchDateRange(day(90), day(92)), '1–3 Apr 1944');
  assert.equal(researchDateRange(day(90), day(90)), '1 Apr 1944');
  assert.equal(researchDateRange(day(364), day(366)), '31 Dec 1944 – 2 Jan 1945');
});
