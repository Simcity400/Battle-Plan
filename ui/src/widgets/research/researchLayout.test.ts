import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { setToken } from '../../dashboard/options.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { deriveTimelineAudit } from '../../families/timeline/model.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BoundKind, ResearchBound, ResearchHistoryReply, ResearchItem, ResearchSlotsReply } from '../../generated/dashboard';
import { researchHistoryProof, validateResearchHistory } from './researchData.ts';
import { researchProjection } from './researchProjection.ts';

const base = HOUR_EPOCH + 1936 * 8760 + 12;
const day = (n: number) => base + n * 24;
const bound = (kind: BoundKind, hour: number): ResearchBound => ({ kind, hour });
const span = (first: number, last: number) => ({ firstHour: first, lastHour: last });
const UNREAD = 'Research was not read for part of this campaign, so more technologies may have been completed';

const item = (token: string, first: number, pieces: { firstHour: number; lastHour: number }[], outcome: ResearchItem['outcome'],
  start: ResearchBound, end: ResearchBound | null, extra: Partial<ResearchItem> = {}): ResearchItem => ({
  token, databaseIndex: 7, firstObservedHour: first, outcome, pieces, start, end, origin: null, tookDays: null, ...extra,
});

const at = day(160);

const replyAt = (hour: number, extra: Partial<ResearchHistoryReply> = {}): ResearchHistoryReply => ({
  hour,
  nextChange: day(170),
  unrecorded: null,
  partial: UNREAD,
  completed: 2,
  coverage: [span(day(0), day(60)), span(day(80), hour)],
  slots: [
    {
      index: 0,
      availability: [span(day(0), day(60)), span(day(80), hour)],
      items: [
        item('tech_alpha', day(0), [span(day(0), day(40) - 1)], 'completed', bound('heldBefore', day(0)), bound('exact', day(40))),
        item('tech_beta', day(40), [span(day(40), day(60)), span(day(80), day(90) - 1)], 'switched', bound('exact', day(40)), bound('by', day(90))),
        item('tech_gamma', day(120), [span(day(120), hour)], 'active', bound('by', day(120)), null),
      ],
    },
    {
      index: 1,
      availability: [span(day(100), day(130)), span(day(140), hour)],
      items: [
        item('tech_delta', day(110), [span(day(110), day(126) - 1)], 'completed', bound('exact', day(110)), bound('exact', day(126)),
          { origin: { slot: 0, start: bound('exact', day(100)) }, tookDays: 26 }),
      ],
    },
  ],
  artwork: [
    { token: 'tech_alpha', name: 'Alpha', description: 'Invented alpha.', baseDurationDays: 100, icon: null },
    { token: 'tech_beta', name: 'Beta', description: null, baseDurationDays: 120, icon: null },
    { token: 'tech_gamma', name: 'Gamma', description: null, baseDurationDays: 150, icon: null },
  ],
  folders: [{ key: 'folder_one', name: 'Folder one' }, { key: 'folder_two', name: 'Folder two' }],
  technologyFolders: [
    { token: 'tech_alpha', databaseIndex: null, folder: 'folder_one' },
    { token: 'tech_gamma', databaseIndex: null, folder: 'folder_two' },
  ],
  definitions: [],
  unnamed: 'The recorded localisation has no name for this technology',
  warnings: [],
  ...extra,
});

const slotsReply = (hour: number): ResearchSlotsReply => ({
  tag: 'AAA',
  readHour: hour,
  missing: null,
  slots: [
    {
      index: 0,
      state: {
        firstHour: hour,
        lastHour: hour,
        token: 'tech_gamma',
        databaseIndex: 7,
        entryFirstObservedHour: day(120),
        speedMultiplierRaw: '100000',
        bankedDaysRaw: '0',
        appliedDaysRaw: '0',
        aheadReductionYearsRaw: '0',
        details: { remainingDays: 29, estimateRaw: '2900000', modifiers: null, bonuses: null },
      },
      progress: { firstHour: hour, lastHour: hour, entryFirstObservedHour: day(120), baseDaysRaw: '10000000', progressRaw: '8100000' },
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

const opt = { folders: '', firstHour: day(0), surface: '#161616' };
const liveMap = new Map([[liveKey('0', 'progress'), 0.81]]);
const viewAt = (hour: number, reply: ResearchHistoryReply, hovers?: Map<string, { hour: number; data: unknown }>) => createBoundedReadView({
  hour,
  panels: new Map<string, unknown>([['main', validateResearchHistory(reply)], ['slots', slotsReply(hour)]]),
  live: new Map([['slots', { get: (key: string) => liveMap.get(key), missing: () => '' }]]),
  ...(hovers ? { hovers } : {}),
});
type Tip = ReturnType<NonNullable<typeof researchProjection.tooltip>>;
const value = (tip: Tip, label: string) => tip?.items?.find(entry => entry.label === label);

test('research lanes draw the product\'s answer at the hour: pieces split at unread stretches, counts and bounds as the product gives them', () => {
  const shown = at + 5;
  const view = viewAt(shown, replyAt(at));
  const model = researchProjection.project(view, opt);

  assert.deepEqual(model.lanes.map(lane => lane.label), ['Slot 1', 'Slot 2']);
  assert.equal(model.summaryText, '2 completed', 'the count is the product\'s, never the client\'s');
  assert.equal(model.summaryPartial, UNREAD, 'a count over an unread stretch carries its reason');
  assert.deepEqual(model.unobserved, [span(day(60) + 1, day(80) - 1)]);
  assert.deepEqual(model.notes, ['Head: 1 in progress · Gamma 81%, 29 days left']);

  const [alpha, betaBefore, betaAfter, gamma] = model.lanes[0]!.spans;
  assert.equal(alpha!.heldBefore, true);
  assert.deepEqual([betaBefore!.lastHour, betaAfter!.firstHour], [day(60), day(80)], 'a block is never drawn through an unread stretch');
  assert.ok(!model.lanes[0]!.spans.some(drawn => drawn.firstHour <= day(70) && day(70) <= drawn.lastHour));
  assert.equal(gamma!.open, true);
  assert.equal(gamma!.startBy, true);
  assert.equal(gamma!.lastHour, shown, 'an open block reaches the displayed hour inside the reply\'s proof');
  assert.deepEqual(model.lanes[1]!.openedMarker, { hour: day(100), label: 'opened' });
  assert.equal(model.nextChangeAt, shown + 24, 'the next day boundary of the days-left estimate comes before the next change of the history');

  const gap = researchProjection.tooltip!(model, { kind: 'lane', laneKey: 'slot:0', hour: day(70) }, view, opt)!;
  assert.equal(value(gap, 'Status')?.value, 'Not recorded');
  const closed = researchProjection.tooltip!(model, { kind: 'lane', laneKey: 'slot:1', hour: day(135) }, view, opt)!;
  assert.equal(value(closed, 'Status')?.value, 'Closed', 'a slot closed between its open stretches is not idle');
  assert.equal(value(closed, 'Closed by')?.value, formatDateOnly(day(130) + 1));

  const switched = researchProjection.tooltip!(model, { kind: 'span', laneKey: 'slot:0', spanId: betaAfter!.id, hour: day(85) }, view, opt)!;
  assert.equal(value(switched, 'Status')?.value, 'Switched');
  assert.equal(value(switched, 'Started')?.value, formatDateOnly(day(40)));
  assert.equal(value(switched, 'Switched')?.value, formatDateOnly(day(90)), 'an end between two reads shows the later read');

  const delta = model.lanes[1]!.spans[0]!;
  assert.equal(delta.label, 'tech_delta', 'an unnamed technology shows its own recorded key');
  const done = researchProjection.tooltip!(model, { kind: 'span', laneKey: 'slot:1', spanId: delta.id, hour: day(115) }, view, opt)!;
  assert.equal(value(done, 'Completed')?.value, formatDateOnly(day(126)));
  assert.equal(value(done, 'Started')?.value, formatDateOnly(day(100)), 'the start is where the technology was first assigned');
  assert.equal(value(done, 'Started in')?.value, 'Slot 1');
  assert.equal(value(done, 'Took')?.value, '26 days');
  assert.match(value(done, 'Name')?.missingReason ?? '', /no name for this technology \(key tech_delta\)/);

  const audit = deriveTimelineAudit(model);
  assert.ok(audit.some(row => row.key === '__notes__' && row.value?.includes('Gamma 81%')), 'the head note is audited');
  assert.ok(audit.some(row => row.key === 'slot:1' && row.value?.includes(`opened:${day(100)}`)), 'the opening marker is audited');
  assert.ok(audit.some(row => row.key === '__summary__' && row.value?.includes('partial:')));

  const filtered = researchProjection.project(view, { ...opt, folders: setToken('folder_two') });
  assert.equal(filtered.lanes[0]!.spans[0]!.color, CHART_MEASURE_COLOURS.other, 'a technology outside the chosen folders is dimmed');
  assert.equal(filtered.lanes[0]!.spans[3]!.color, CHART_MEASURE_COLOURS.intelCoverage);
});

test('missing research is said with its reason; an empty campaign is said only when fully read', () => {
  const unrecorded = researchProjection.project(viewAt(at, replyAt(at, { unrecorded: 'Research was not recorded by this date', slots: [], coverage: [] })), opt);
  assert.equal(unrecorded.emptyKind, 'unavailable');
  assert.equal(unrecorded.empty, 'Research was not recorded by this date.');
  const partial = researchProjection.project(viewAt(at, replyAt(at, { slots: [] })), opt);
  assert.equal(partial.emptyKind, 'unavailable', 'no slots over an unread stretch is not "no slots"');
  const empty = researchProjection.project(viewAt(at, replyAt(at, { slots: [], partial: null })), opt);
  assert.equal(empty.emptyKind, 'empty');
  assert.throws(() => validateResearchHistory({ ...replyAt(at), coverage: [span(day(0), at + 1)] }), /does not match/, 'nothing after the answered hour');
  assert.deepEqual(researchHistoryProof(replyAt(at), [day(0), at], at + 3), [at, day(170) - 1]);
  assert.equal(researchHistoryProof(replyAt(at), [day(0), at], day(170)), null);
});

test('a hover at an earlier hour of an open block or an idle slot reads the slots at the hit hour, never the playhead', () => {
  const earlier = slotsReply(day(121));
  const hovered: ResearchSlotsReply = {
    ...earlier,
    slots: [
      {
        ...earlier.slots[0]!,
        state: { ...earlier.slots[0]!.state, details: { remainingDays: 66, estimateRaw: '6600000', modifiers: null, bonuses: null } },
        progress: { ...earlier.slots[0]!.progress!, progressRaw: '3000000' },
      },
      { ...earlier.slots[1]!, state: { ...earlier.slots[1]!.state, bankedDaysRaw: '200000' } },
    ],
  };
  const model = researchProjection.project(viewAt(at, replyAt(at)), opt);
  const gamma = model.lanes[0]!.spans[3]!;

  const pending = researchProjection.tooltip!(model, { kind: 'span', laneKey: 'slot:0', spanId: gamma.id, hour: day(121) }, viewAt(at, replyAt(at)), opt)!;
  assert.equal(pending.hour, day(121));
  assert.notEqual(value(pending, 'Progress')?.value, '81%', 'the playhead progress is never labelled with an earlier hour');
  assert.notEqual(value(pending, 'Days left')?.value, '29', 'the playhead estimate is never labelled with an earlier hour');
  const idlePending = researchProjection.tooltip!(model, { kind: 'lane', laneKey: 'slot:1', hour: day(145) }, viewAt(at, replyAt(at)), opt)!;
  assert.equal(value(idlePending, 'Saved progress')?.value, undefined, 'saved days at the playhead are not shown for an earlier hour');

  const answered = viewAt(at, replyAt(at), new Map([['hover', { hour: day(121), data: hovered }]]));
  const tip = researchProjection.tooltip!(model, { kind: 'span', laneKey: 'slot:0', spanId: gamma.id, hour: day(121) }, answered, opt)!;
  assert.equal(value(tip, 'Progress')?.value, '30%');
  assert.equal(value(tip, 'Days left')?.value, '66');
  const idleAnswered = viewAt(at, replyAt(at), new Map([['hover', { hour: day(145), data: { ...hovered, readHour: day(145) } }]]));
  const idle = researchProjection.tooltip!(model, { kind: 'lane', laneKey: 'slot:1', hour: day(145) }, idleAnswered, opt)!;
  assert.equal(value(idle, 'Saved progress')?.value, '2 days saved');

  const playhead = researchProjection.tooltip!(model, { kind: 'span', laneKey: 'slot:0', spanId: gamma.id, hour: at }, answered, opt)!;
  assert.equal(value(playhead, 'Progress')?.value, '81%', 'at the playhead the live progress stays');
  assert.equal(value(playhead, 'Days left')?.value, '29');
});
