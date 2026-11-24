import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { daysLeft, decisionAt, decisionCard, lifecycleLine, STATUS_CHOICES, statusLabel } from './decisionsData.ts';
import { plainScripted, scriptedParts } from '../event-feed/scriptedGaps.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import { decisionsProjection } from './decisionsProjection.ts';
import type { EventFeedKind, EventFeedRow, ScriptCost, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';

const day = (month: number, date: number, year = 1939) => engineHour({ year, month, day: date, hour: 1 })!;
let serial = 0;
const edge = (kind: EventFeedKind, hour: number, extra: Partial<ScriptEdge> = {}, participants: [string | null, string | null] = ['GER', null]): ScriptEdge => ({
  row: { id: `native:2:${serial++}`, hour, readHour: hour, nameHour: hour, exact: true, category: 'politics', kind,
    tags: participants.filter((tag): tag is string => tag !== null), participants,
    item: { token: 'mod_decision', name: 'Modded decision', icon: null, description: null }, amount: null } as EventFeedRow,
  option: null, lifecycle: null, days: null, costs: null, ...extra });
const card = (edges: ScriptEdge[], opened = true): ScriptOccurrence => ({ id: edges[0].row.id, opened, edges });
const at = (edges: ScriptEdge[], hour: number, breaks: { firstHour: number; lastHour: number }[] = [], opened = true) => {
  const held = decisionCard(card(edges, opened));
  const first = held?.occurrence.edges[0].row;
  return decisionAt({ key: '1', id: held?.occurrence.id ?? '1', hour: first?.hour ?? edges[0].row.readHour, revealedHour: first?.readHour ?? edges[0].row.readHour, by: false, item: held }, hour, breaks)?.item ?? null;
};
const pp = (amount: string): ScriptCost[] => [{ currency: 'politicalPower', amount }];

test('a decision\'s status moves in place with the hour shown: a timer counting down, then its end', () => {
  const start = day(3, 1);
  const timed = [edge('decisionSelected', start, { days: 70, costs: pp('-5000000') }), edge('decisionRemoved', day(5, 10))];
  const running = at(timed, start + 58 * 24)!;
  assert.deepEqual([statusLabel(running), lifecycleLine(running), daysLeft(running)], ['Active', '70-day timer · 12 days left', 12]);
  const removed = at(timed, day(6, 1))!;
  assert.deepEqual([statusLabel(removed), lifecycleLine(removed)], ['Removed', 'Removed by the game']);
  assert.equal(at(timed, start - 1), null, 'not read yet');
});

test('a running decision across a break in the recording has no end recorded, never a guessed one', () => {
  const start = day(3, 1);
  const mission = [edge('decisionActivated', start, { days: 20 })];
  const unknown = at(mission, day(5, 1), [{ firstHour: start + 100, lastHour: start + 200 }])!;
  assert.deepEqual([unknown.status, statusLabel(unknown)], ['unknown', 'Not recorded']);
  assert.match(lifecycleLine(unknown), /no end recorded by/);
});

test('a step whose selection was not recorded is its own row', () => {
  const alone = at([edge('decisionCompleted', day(4, 1))], day(4, 2), [], false)!;
  assert.deepEqual([alone.start, alone.status], [null, 'completed']);
});

test('a decision title operand the recording cannot prove is a "not recorded" token with its reason, never a bare ellipsis', () => {
  const why = 'decision title seize_gold: it names a highlighted state the recording does not hold';
  const item = { token: 'seize_gold', name: 'Seize the Gold Reserve of \uFFFC', icon: null, description: null, gaps: why };
  assert.equal(plainScripted(item.name), 'Seize the Gold Reserve of [not recorded]');
  assert.deepEqual(scriptedParts(item.name, why), [{ text: 'Seize the Gold Reserve of ' }, { gap: `Not recorded: ${why}` }]);
  assert.deepEqual(scriptedParts('Recruit in …', why), [{ text: 'Recruit in …' }], 'an authored ellipsis is text');
});

test('a mission with a recorded Failed native state shows Failed with its observation date while keeping its witnessed cause and filter choice', () => {
  assert.ok(STATUS_CHOICES.some(choice => choice.value === 'failed' && choice.label === 'Failed'));
  const start = day(3, 1);
  const end = day(5, 10);
  const timedOutFailed = [
    edge('decisionActivated', start, { days: 70 }),
    edge('decisionTimedOut', end, { row: { ...edge('decisionTimedOut', end).row, decisionState: { kind: 'failed', observedHour: end } } }),
  ];
  const shown = at(timedOutFailed, day(6, 1))!;
  assert.equal(shown.status, 'failed');
  assert.equal(statusLabel(shown), 'Failed');
  assert.equal(lifecycleLine(shown), 'Failed · Timed out after 70 days');

  const noInventedCause = [
    edge('decisionActivated', start, { days: 30 }),
    edge('decisionCompleted', end, { row: { ...edge('decisionCompleted', end).row, decisionState: { kind: 'failed', observedHour: end } } }),
  ];
  const failedNoTimeout = at(noInventedCause, day(6, 1))!;
  assert.equal(failedNoTimeout.status, 'failed');
  assert.match(lifecycleLine(failedNoTimeout), /Activated .* → failed /);
  assert.doesNotMatch(lifecycleLine(failedNoTimeout), /Timed out|Cancelled/);

  const notRecorded = [
    edge('decisionActivated', start, { days: 30 }),
    edge('decisionTimedOut', end, { row: { ...edge('decisionTimedOut', end).row, decisionState: { kind: 'notRecorded', reason: 'mission failure state was not recorded' } } }),
  ];
  const withheld = at(notRecorded, day(6, 1))!;
  assert.equal(withheld.status, 'timedOut');
  assert.match(lifecycleLine(withheld), /mission failure state was not recorded/);

  const instant = [
    edge('decisionSelected', start, { row: { ...edge('decisionSelected', start).row, decisionState: { kind: 'notApplicable', reason: 'decision has no mission lifecycle state' } } }),
  ];
  const instantShown = at(instant, day(6, 1))!;
  assert.equal(instantShown.status, 'taken');
  assert.equal(instantShown.failureMissing, null);

  const unsupported = [
    edge('decisionActivated', start, { days: 30 }),
    edge('decisionTimedOut', end, { row: { ...edge('decisionTimedOut', end).row, decisionState: { kind: 'unsupported', raw: 99 } } }),
  ];
  const unknownNative = at(unsupported, day(6, 1))!;
  assert.equal(unknownNative.status, 'unsupported');
  assert.equal(statusLabel(unknownNative), 'Unsupported (state 99)');

  const completedLifecycle = at([
    edge('decisionActivated', start, { days: 30, row: { ...edge('decisionActivated', start).row, decisionState: { kind: 'completed' } } }),
  ], day(4, 1))!;
  assert.equal(completedLifecycle.status, 'completed');
  assert.match(lifecycleLine(completedLifecycle), /completed/);

  const abortedLifecycle = at([
    edge('decisionActivated', start, { days: 30, row: { ...edge('decisionActivated', start).row, decisionState: { kind: 'aborted' } } }),
  ], day(4, 1))!;
  assert.equal(abortedLifecycle.status, 'cancelled');
  assert.match(lifecycleLine(abortedLifecycle), /aborted/);

  const selfTargeted = decisionCard(card([edge('decisionSelected', start, {}, ['GER', 'GER'])]));
  assert.equal(selfTargeted?.country, 'GER');
  assert.equal(selfTargeted?.target, null, 'same holder and target produces one flag, never two');
});

test('decisions projection formats unsigned costs with PP icon, keeps unknown chargeable edges missing, omits cost on missions, and bounds timer validity', () => {
  const start = day(3, 1);
  const hour = start + 10;
  const missionOcc = card([edge('decisionActivated', start, { days: 10 })]);
  const mixedUnknownOcc = card([
    edge('decisionSelected', start, { costs: null }),
    edge('decisionTimeoutEffect', start + 2, { costs: [] }),
  ]);
  const chargedOcc = card([edge('decisionSelected', start, { days: 10, costs: pp('-5000000') })]);
  const head = {
    branchId: '1',
    log: 'feed-events',
    columns: [],
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '3',
    counts: [{ key: 'running', total: '2' }],
    rows: [
      { hour: start, key: 'k3', ordinal: '2', cells: [], occurrence: chargedOcc },
      { hour: start, key: 'k2', ordinal: '1', cells: [], occurrence: mixedUnknownOcc },
      { hour: start, key: 'k1', ordinal: '0', cells: [], occurrence: missionOcc },
    ],
    facetTotals: [],
    missing: [],
    unavailable: [],
    warnings: [],
  };
  const source = buildRowSourceFromHead('test:decisions', head, undefined, hour)!;
  const readView = createBoundedReadView({
    hour,
    rows: new Map([['main', source]]),
    art: { resolve: () => ({ status: 'ready', assetId: 'pp', url: 'art://pp', origin: 'vanilla', file: 'pp.dds', frame: 0, provenance: 'recorded' }) as never },
  });
  const model = decisionsProjection.project(readView, {
    artwork: true,
    breaks: [],
  });
  assert.equal(model.titleSummary, '2 running');
  assert.equal(model.nextChangeAt, start + 24);
  const byKey = new Map(model.rows.map(r => [r.key, r]));
  const chargedRow = byKey.get(chargedOcc.id)!;
  assert.deepEqual(chargedRow.cells.cost?.value, { kind: 'known', value: '50' });
  assert.equal(chargedRow.cells.cost?.unitSuffix, 'PP');
  assert.equal(chargedRow.cells.cost?.unitIcon, 'art://pp');
  assert.ok(chargedRow.meta, 'decision row carries detail subject');
  const mixedRow = byKey.get(mixedUnknownOcc.id)!;
  assert.equal(mixedRow.cells.cost?.value.kind, 'notRecorded', 'unknown selection cost never collapses to No cost');
  const missionRow = byKey.get(missionOcc.id)!;
  assert.equal(missionRow.cells.cost?.value.kind, 'notApplicable');
  assert.equal(missionRow.cells.cost?.blank, true);

  const untimedActiveOcc = card([edge('decisionActivated', start, { days: null })]);
  const untimedSource = buildRowSourceFromHead('test:decisions-untimed', {
    ...head,
    total: '1',
    rows: [{ hour: start, key: 'k-untimed', ordinal: '0', cells: [], occurrence: untimedActiveOcc }],
  }, undefined, hour)!;
  const untimedModel = decisionsProjection.project(
    createBoundedReadView({ hour, rows: new Map([['main', untimedSource]]) }),
    { artwork: false, breaks: [] },
  );
  assert.equal(untimedModel.rows[0]?.cells.timer?.value.kind, 'notRecorded', 'active row without duration shows Duration not recorded, never blank');
  assert.equal(untimedModel.rows[0]?.cells.timer?.missingFigure, 'Duration');
});
