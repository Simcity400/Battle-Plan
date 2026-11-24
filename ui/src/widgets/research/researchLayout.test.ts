import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { setToken } from '../../dashboard/options.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { ResearchEntry, ResearchSlotState, ResearchSlotsReply } from '../../generated/dashboard';
import { completedRow, coverageAt, coverageGaps } from './researchLayout.ts';
import { researchProjection } from './researchProjection.ts';
import type { ResearchData, ResearchHistory, ResearchSlotHistory } from './types';

const base = HOUR_EPOCH + 1936 * 8760 + 12;
const day = (n: number) => base + n * 24;

const entry = (
  token: string,
  first: number,
  end: number | null,
  outcome: ResearchEntry['outcome'] = 'completed',
  startedAfter: number | null = first - 24,
): ResearchEntry => ({
  token,
  technologyHandle: token.charCodeAt(0),
  databaseIndex: token.charCodeAt(0),
  continuityStartHour: day(0),
  level: 0,
  firstObservedHour: first,
  startedAfterHour: startedAfter,
  lastObservedHour: end === null ? first : end - 24,
  endedByHour: end,
  outcomeObservedHour: null,
  outcome,
});

const slot = (index: number, availability: { firstHour: number; lastHour: number }[], entries: ResearchEntry[], states: ResearchSlotState[] = []): ResearchSlotHistory => ({
  index,
  availability,
  entries,
  states,
  progress: [],
});

const dataOf = (history: ResearchHistory): ResearchData => ({
  history,
  artwork: [
    { token: 'radio', name: 'Radio', description: 'Enables radio sets.', baseDurationDays: 100, icon: null },
    { token: 'mech', name: 'Mechanical Computing', description: null, baseDurationDays: 120, icon: null },
    { token: 'sub', name: 'Improved Submarine', description: 'Modern hull.', baseDurationDays: 150, icon: null },
    { token: 'tank', name: 'Basic Medium Tank', description: null, baseDurationDays: 160, icon: null },
  ],
  folders: [
    { key: 'electronics_folder', name: 'Electronics' },
    { key: 'naval_folder', name: 'Naval' },
  ],
  technologyFolders: [
    { token: 'radio', databaseIndex: null, folder: 'electronics_folder' },
    { token: 'mech', databaseIndex: null, folder: 'electronics_folder' },
    { token: 'sub', databaseIndex: null, folder: 'naval_folder' },
  ],
  definitions: [],
  warnings: [],
});

const slotsReply = (hour: number): ResearchSlotsReply => ({
  tag: 'USA',
  readHour: hour,
  missing: null,
  slots: [
    {
      index: 0,
      state: {
        firstHour: hour,
        lastHour: hour,
        token: 'sub',
        databaseIndex: 's'.charCodeAt(0),
        entryFirstObservedHour: day(120),
        speedMultiplierRaw: '100000',
        bankedDaysRaw: '0',
        appliedDaysRaw: '0',
        aheadReductionYearsRaw: '0',
        details: { remainingDays: 29, estimateRaw: '2900000', modifiers: null, bonuses: null },
      },
      progress: {
        firstHour: hour,
        lastHour: hour,
        entryFirstObservedHour: day(120),
        baseDaysRaw: '10000000',
        progressRaw: '8100000',
      },
      definition: null,
    },
    {
      index: 1,
      state: {
        firstHour: hour,
        lastHour: hour,
        token: null,
        databaseIndex: null,
        entryFirstObservedHour: null,
        speedMultiplierRaw: '100000',
        bankedDaysRaw: '1250000',
        appliedDaysRaw: '0',
        aheadReductionYearsRaw: null,
        details: null,
      },
      progress: null,
      definition: null,
    },
  ],
  grants: { nextId: null, grants: [], unavailable: null },
  warnings: [],
});

test('research timeline projects slot lanes clipped to the playhead, with coverage gaps, head note, folder dimming, and tooltips', () => {
  const history: ResearchHistory = {
    coverage: [{ firstHour: day(0), lastHour: day(60) }, { firstHour: day(80), lastHour: day(200) }],
    slots: [
      slot(0, [{ firstHour: day(0), lastHour: day(60) }, { firstHour: day(80), lastHour: day(200) }], [
        entry('radio', day(0), day(40), 'completed', null),
        entry('mech', day(40), day(90), 'switched'),
        entry('sub', day(120), day(220), 'completed'),
      ]),
      slot(1, [{ firstHour: day(100), lastHour: day(200) }], [
        entry('tank', day(110), day(150), 'completed'),
      ]),
      slot(2, [{ firstHour: day(180), lastHour: day(200) }], []),
    ],
  };

  const at = day(160);
  const data = dataOf(history);
  const liveMap = new Map([[liveKey('0', 'progress'), 0.81]]);
  const view = createBoundedReadView({
    hour: at,
    panels: new Map<string, unknown>([['main', data], ['slots', slotsReply(at)]]),
    live: new Map([['slots', { get: (key: string) => liveMap.get(key), missing: () => '' }]]),
  });
  const opt = { folders: '', firstHour: day(0), surface: '#161616' };
  const model = researchProjection.project(view, opt);

  assert.equal(model.lanes.length, 2, 'slot 3 opens at day 180 and stays hidden at day 160');
  assert.deepEqual(model.lanes.map(l => l.label), ['Slot 1', 'Slot 2']);
  assert.equal(model.lanes[0]!.openedMarker, undefined, 'slot open at coverage start carries no opened marker');
  assert.deepEqual(model.lanes[1]!.openedMarker, { hour: day(100), label: 'opened' }, 'slot opening inside recorded coverage carries opened marker');
  assert.deepEqual(model.legend?.map(entry => entry.label), ['Completed', 'In progress', 'Idle', 'Not recorded']);
  assert.equal(model.summaryText, '2 completed');
  assert.deepEqual(model.unobserved, [{ firstHour: day(60) + 1, lastHour: day(80) - 1 }]);
  assert.deepEqual(model.notes, ['Head: 1 in progress · Improved Submarine 81%, 29 days left']);

  const [radio, mech, sub] = model.lanes[0]!.spans;
  assert.equal(radio!.label, 'Radio');
  assert.equal(radio!.open, false);
  assert.equal(radio!.heldBefore, true);
  assert.equal(radio!.endUnseen, undefined, 'daily-sampled completion inside recorded coverage has solid right edge');
  assert.equal(radio!.color, CHART_MEASURE_COLOURS.intelCoverage);
  assert.equal(mech!.color, CHART_MEASURE_COLOURS.mutedFallback, 'switched entries use muted colour');
  assert.equal(sub!.label, 'Improved Submarine');
  assert.equal(sub!.open, true, 'completion at day 220 is future at day 160 so span stays open');
  assert.equal(sub!.lastHour, at);

  const activeTip = researchProjection.tooltip!(model, { kind: 'span', laneKey: 'slot:0', spanId: sub!.id, hour: at }, view, opt)!;
  assert.equal(activeTip.title, 'Improved Submarine');
  assert.equal(activeTip.subtitle, 'Naval · Slot 1');
  assert.ok(activeTip.items?.some(item => item.label === 'Progress' && item.value === '81%'));
  assert.ok(activeTip.items?.some(item => item.label === 'Days left' && item.value === '29'));

  const completedTip = researchProjection.tooltip!(model, { kind: 'span', laneKey: 'slot:0', spanId: radio!.id, hour: day(20) }, view, opt)!;
  assert.equal(completedTip.title, 'Radio');
  assert.equal(completedTip.description, 'Enables radio sets.');
  assert.ok(completedTip.items?.some(item => item.label === 'Completed'));

  const unopenedTip = researchProjection.tooltip!(model, { kind: 'lane', laneKey: 'slot:1', hour: day(20) }, view, opt)!;
  assert.ok(unopenedTip.items?.some(item => item.label === 'Status' && item.value === 'Not yet opened'));

  const gapTip = researchProjection.tooltip!(model, { kind: 'lane', laneKey: 'slot:0', hour: day(70) }, view, opt)!;
  assert.ok(gapTip.items?.some(item => item.label === 'Status' && item.value === 'Not recorded'));

  const filtered = researchProjection.project(view, { ...opt, folders: setToken('naval_folder') });
  assert.equal(filtered.lanes[0]!.spans[0]!.color, CHART_MEASURE_COLOURS.other, 'non-matching folder dimmed');
  assert.equal(filtered.lanes[0]!.spans[2]!.color, CHART_MEASURE_COLOURS.intelCoverage, 'matching folder stays highlighted');
  assert.equal(filtered.summaryText, '0 completed');

  const noGapData = dataOf({
    coverage: [{ firstHour: day(0), lastHour: day(200) }],
    slots: [slot(0, [{ firstHour: day(0), lastHour: day(200) }], [])],
  });
  const noGapModel = researchProjection.project(createBoundedReadView({ hour: day(100), panels: new Map<string, unknown>([['main', noGapData]]) }), opt);
  assert.deepEqual(noGapModel.legend?.map(entry => entry.label), ['Completed', 'In progress', 'Idle']);

  const afterGapData = dataOf({
    coverage: [{ firstHour: day(0), lastHour: day(60) }, { firstHour: day(80), lastHour: day(200) }],
    slots: [
      slot(0, [{ firstHour: day(0), lastHour: day(60) }, { firstHour: day(80), lastHour: day(200) }], []),
      slot(1, [{ firstHour: day(80), lastHour: day(200) }], []),
    ],
  });
  const afterGapModel = researchProjection.project(createBoundedReadView({ hour: day(120), panels: new Map<string, unknown>([['main', afterGapData]]) }), opt);
  assert.equal(afterGapModel.lanes[1]!.openedMarker, undefined, 'slot first seen right after a gap has unrecorded opening and omits marker');
});

test('coverage helpers and completedRow preserve cross-slot start date and duration', () => {
  assert.deepEqual(coverageAt([{ firstHour: day(0), lastHour: day(100) }], day(40)), [{ firstHour: day(0), lastHour: day(40) }]);
  assert.deepEqual(coverageGaps([{ firstHour: day(0), lastHour: day(20) }, { firstHour: day(40), lastHour: day(60) }]), [{ firstHour: day(20) + 1, lastHour: day(40) - 1 }]);

  const opening: ResearchSlotState = {
    firstHour: day(10),
    lastHour: day(40) - 1,
    token: 'T',
    databaseIndex: 'T'.charCodeAt(0),
    entryFirstObservedHour: day(10),
    speedMultiplierRaw: '100000',
    bankedDaysRaw: '0',
    appliedDaysRaw: '0',
    aheadReductionYearsRaw: '0',
    details: null,
  };
  const slots: ResearchSlotHistory[] = [
    {
      ...slot(0, [{ firstHour: day(0), lastHour: day(200) }], [entry('T', day(10), day(40), 'switched')], [opening]),
      progress: [{ firstHour: day(10), lastHour: day(11) - 1, entryFirstObservedHour: day(10), baseDaysRaw: '10000000', progressRaw: '0' }],
    },
    slot(1, [{ firstHour: day(0), lastHour: day(200) }], [entry('T', day(40), day(90))]),
  ];
  const row = completedRow(slots, slots[1]!, slots[1]!.entries[0]!, day(90));
  assert.equal(row.startSlot, 0);
  assert.equal(row.started.hour, day(10));
  assert.equal(row.tookHours, day(90) - day(10));
});
