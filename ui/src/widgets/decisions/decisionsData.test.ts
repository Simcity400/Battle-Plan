import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { daysLeft, decisionAt, decisionCard, lifecycleLine, STATUS_CHOICES, statusLabel } from './decisionsData.ts';
import { plainScripted, scriptedParts } from '../event-feed/scriptedGaps.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead } from '../../families/table/rowSource.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { resolveSelectedCountryRefs } from '../../families/table/logHelpers.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { decisionsDeclaration, decisionsProjection } from './decisionsProjection.ts';
import { hoverTextOf, withHoverText, type HoverText } from '../event-feed/hoverText.ts';
import { eventChoicesDeclaration } from '../event-choices/eventChoicesProjection.ts';
import { eventFeedDeclaration } from '../event-feed/eventFeedProjection.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import type { CountrySelection } from '../../dashboard/model.ts';
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
  return decisionAt(held, hour, breaks);
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
  const gap = { field: 'title' as const, key: 'FROM.GetName', reason: 'it names a highlighted state the recording does not hold' };
  const item = { token: 'synth_gold', name: 'Reserve of \uFFFC and \uFFFC', icon: null, description: null, gaps: [gap, { field: 'title' as const, key: 'ROOT.GetLeader', reason: 'reads more than a country name or date' }] };
  assert.equal(plainScripted(item.name, item.gaps), 'Reserve of [not recorded: FROM.GetName: it names a highlighted state the recording does not hold] and [not recorded: ROOT.GetLeader: reads more than a country name or date]');
  assert.deepEqual(scriptedParts(item.name, item.gaps), [
    { text: 'Reserve of ' },
    { gap: 'Not recorded: FROM.GetName: it names a highlighted state the recording does not hold', inline: 'not recorded: FROM.GetName: it names a highlighted state the recording does not hold', key: 'FROM.GetName', reason: 'it names a highlighted state the recording does not hold' },
    { text: ' and ' },
    { gap: 'Not recorded: ROOT.GetLeader: reads more than a country name or date', inline: 'not recorded: ROOT.GetLeader: reads more than a country name or date', key: 'ROOT.GetLeader', reason: 'reads more than a country name or date' },
  ]);
  assert.deepEqual(scriptedParts('Recruit in …', item.gaps), [{ text: 'Recruit in …' }], 'an authored ellipsis is text');
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
  const { costs: _absent, ...unproven } = edge('decisionSelected', start);
  const mixedUnknownOcc = card([
    unproven as ScriptEdge,
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
  assert.equal(chargedRow.cells.cost?.unitArt?.status === 'ready' ? chargedRow.cells.cost.unitArt.url : null, 'art://pp');
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

  const productStatusSource = buildRowSourceFromHead('test:decisions-status', {
    ...head,
    total: '2',
    counts: [{ key: 'running', total: '1' }],
    rows: [
      { hour: start, key: 'k-live', ordinal: '0', live: true, status: 'active', occurrence: chargedOcc },
      { hour: start, key: 'k-nr', ordinal: '1', live: false, status: 'notRecorded', occurrence: missionOcc },
    ],
  }, undefined, hour)!;
  const productStatusModel = decisionsProjection.project(
    createBoundedReadView({ hour, rows: new Map([['main', productStatusSource]]) }),
    { artwork: false, breaks: [] },
  );
  assert.equal(productStatusModel.rows[0]?.live, true);
  assert.equal(productStatusModel.rows[1]?.live, false);
  assert.deepEqual(productStatusModel.rows[1]?.cells.status?.value, { kind: 'known', value: 'Not recorded' });
});

test('R2-S1: real filter-mode log declarations send selected and explicit country keys instead of empty countries, pass groups directly, and omit selection only for all', () => {
  const planAsk = (declaration: typeof decisionsDeclaration | typeof eventChoicesDeclaration | typeof eventFeedDeclaration, countries: CountrySelection) => {
    const ctx = {
      track: { source: { id: 's1', revision: 'r1', metadata: { playerCountryId: '9' } }, axisOffset: 0, selectedSlot: 7 },
      branch: { id: '1', firstHour: 0, lastHour: 200, gaps: [] },
      widget: { id: 'w1', kind: declaration.kind, countries, time: { mode: 'follow' } },
      options: { warsPeace: true },
      following: true,
      countries: {
        all: countries.kind === 'all',
        tags: [],
        filter: { passing: null, unrecorded: [], bounds: [] },
      },
      gateSpan: (firstHour: number, lastHour: number) => ({ firstHour, lastHour }),
    } as unknown as FamilyPlanContext<unknown>;
    const entry = declaration.sources(ctx, { following: true, staticSpan: null }).entries[0]!;
    if (entry.read.shape !== 'paged-rows') throw new Error('expected paged-rows');
    return entry.read.tailQuery.ask as { countries?: unknown; group?: unknown };
  };

  for (const declaration of [decisionsDeclaration, eventChoicesDeclaration, eventFeedDeclaration]) {
    assert.deepEqual(planAsk(declaration, { kind: 'tags', tags: ['GER'] }).countries, ['GER'], `${declaration.kind}: one explicit country`);
    assert.deepEqual(planAsk(declaration, { kind: 'tags', tags: ['GER', 'ITA'] }).countries, ['GER', 'ITA'], `${declaration.kind}: two explicit countries`);
    assert.deepEqual(planAsk(declaration, { kind: 'selected' }).countries, [7], `${declaration.kind}: selected country sends the track slot`);
    assert.deepEqual(planAsk(declaration, { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }).group, { type: 'humans', membership: 'hour' }, `${declaration.kind}: groups pass directly`);
    assert.equal(planAsk(declaration, { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }).countries, undefined);
    const all = planAsk(declaration, { kind: 'all' });
    assert.equal(all.countries, undefined, `${declaration.kind}: all omits countries`);
    assert.equal(all.group, undefined, `${declaration.kind}: all omits group`);
    assert.deepEqual(planAsk(declaration, { kind: 'tags', tags: [] }).countries, [], `${declaration.kind}: explicit empty tags stays empty selection`);
  }
});

test('S16: explicit tag selection resolves to recorded slots while keeping empty distinct from all', () => {
  const identities = countryIdentityIndex([
    { tag: 'AAA', identities: [{ firstHour: 100, lastHour: 150, slot: 7, countryId: 9001 }] },
    { tag: 'BBB', identities: [{ firstHour: 151, lastHour: 200, slot: 7, countryId: 9001 }] },
  ]);
  const resolvedSlots = resolveSelectedCountryRefs({ kind: 'tags', tags: ['AAA'] }, ['AAA'], identities);
  assert.deepEqual(resolvedSlots, [7, 'AAA'], 'saved tag AAA keeps resolved slot 7 together with tag AAA so Rust reports unknown-holder gaps');
  assert.deepEqual(resolveSelectedCountryRefs({ kind: 'selected' }, [countryKey(7)], identities), [7], 'explicitly selected slot sends only the slot');
  assert.deepEqual(resolveSelectedCountryRefs({ kind: 'tags', tags: [] }, [], identities), [], 'empty explicit selection stays []');
  assert.equal(resolveSelectedCountryRefs({ kind: 'all' }, [], identities), undefined, 'all returns undefined');
  assert.deepEqual(resolveSelectedCountryRefs({ kind: 'tags', tags: ['ZZZ'] }, ['ZZZ'], identities), ['ZZZ'], 'unresolved tag keeps its tag so Rust can report unknown-holder gaps');

  const ctx = {
    track: { source: { id: 's1', revision: 'r1', metadata: { playerCountryId: '9' } }, axisOffset: 0, selectedSlot: 7 },
    branch: { id: '1', firstHour: 0, lastHour: 200, gaps: [] },
    widget: { id: 'w1', kind: 'decisions', countries: { kind: 'tags', tags: ['AAA'] }, time: { mode: 'follow' } },
    options: {},
    following: true,
    countries: {
      all: false,
      tags: [],
      selectedSlots: resolvedSlots,
      filter: { passing: null, unrecorded: [], bounds: [] },
    },
    gateSpan: (firstHour: number, lastHour: number) => ({ firstHour, lastHour }),
  } as unknown as FamilyPlanContext<unknown>;
  const entry = decisionsDeclaration.sources(ctx, { following: true, staticSpan: null }).entries[0]!;
  if (entry.read.shape !== 'paged-rows') throw new Error('expected paged-rows');
  const ask = entry.read.tailQuery.ask as { countries?: unknown };
  assert.deepEqual(ask.countries, [7, 'AAA'], 'saved tag AAA sends both slot 7 (preserving renamed-tag history) and tag AAA (preserving unknown-holder coverage)');
});

test('S20: frozen Decisions page derives row state, timer, and detail asOfHour from source.at rather than playhead hour', () => {
  const start = day(3, 1);
  const frozenAt = start + 24;
  const playhead = start + 72;
  const occ = card([edge('decisionSelected', start, { days: 3 })]);
  const rawPage = {
    branchId: '1',
    log: 'feed-events',
    columns: [],
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '1',
    counts: [{ key: 'running', total: '1' }],
    rows: [{ hour: start, key: 'k-frozen', ordinal: '0', live: true, status: 'active', occurrence: occ }],
    facetTotals: [],
    missing: [],
    unavailable: [],
    warnings: [],
  };
  const pages = {
    0: {
      hour: frozenAt,
      offset: 0,
      limit: 256,
      raw: rawPage,
      reply: {
        total: 1,
        offset: 0,
        baseOrdinal: 0,
        firstOrdinal: 0,
        rows: rawPage.rows as never,
        facetTotals: new Map([['running', 1]]),
        missing: [],
      },
    },
  };
  const source = buildRowSourceFromHead('test:decisions-frozen', undefined, undefined, playhead, pages)!;
  assert.equal(source.at, frozenAt);
  const model = decisionsProjection.project(
    createBoundedReadView({ hour: playhead, rows: new Map([['main', source]]) }),
    { artwork: false, breaks: [] },
  );
  assert.equal(model.rows.length, 1);
  const row = model.rows[0]!;
  assert.deepEqual(row.cells.timer?.value, { kind: 'known', value: '2' }, 'timer must use frozen snapshot hour (2 days left), not playhead (0 days left)');
  assert.equal((row.meta as { asOfHour: number }).asOfHour, frozenAt, 'detail asOfHour must match frozen snapshot hour');
});

test('a decision tooltip shows its listed description until the hovered row is read, then the full text, and says what could not be read', () => {
  const start = day(3, 1);
  const selected = edge('decisionSelected', start, { days: 10 });
  const occ = card([{ ...selected, row: { ...selected.row, item: { ...selected.row.item!, description: 'Raise war bonds.' } } }]);
  const other = card([edge('decisionSelected', start, { days: 10 })]);
  const source = buildRowSourceFromHead('test:decisions-hover', {
    branchId: '1', log: 'feed-events', columns: [], baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: '2', counts: [],
    rows: [{ hour: start, key: 'a', ordinal: '0', cells: [], occurrence: occ }, { hour: start, key: 'b', ordinal: '1', cells: [], occurrence: other }],
    facetTotals: [], missing: [], unavailable: [], warnings: [],
  }, undefined, start + 1)!;
  const tip = (hover: HoverText | readonly HoverText[] | null) => {
    const model = decisionsProjection.project(createBoundedReadView({ hour: start + 1, rows: new Map([['main', source]]) }), { artwork: false, breaks: [], hover: hoverTextOf(withHoverText({}, hover === null ? [] : 'key' in hover ? [hover] : hover)) });
    const extra = model.rows.find(row => row.key === occ.id)?.tooltipExtra;
    const shown = (typeof extra === 'function' ? extra() : extra) ?? [];
    return shown.filter(row => row.label.startsWith('Text not recorded') || ['Description', 'Full description', 'Authored effects'].includes(row.label))
      .map(row => [row.label, row.value.kind === 'known' ? row.value.value : row.value.kind === 'notRecorded' ? `not recorded: ${row.value.reason}` : row.value.kind]);
  };
  const read: HoverText = { key: occ.id, description: 'Raise war bonds. Effects: +5% war support', gaps: null, options: null, missing: null };
  assert.deepEqual(tip(null), [['Description', 'Raise war bonds.']], 'before the read answers, the listed description');
  assert.deepEqual(tip(read), [['Description', 'Raise war bonds. Effects: +5% war support']], 'the read row brings the full text');
  assert.deepEqual(tip({ ...read, key: other.id }), [['Description', 'Raise war bonds.']], 'a read of another row changes nothing here');
  assert.deepEqual(tip([{ ...read, key: other.id, description: 'Elsewhere.' }, read]), [['Description', 'Raise war bonds. Effects: +5% war support']], 'with a pointer row and a focused row read, each tooltip takes its own');
  assert.deepEqual(tip({ ...read, description: null, missing: 'This event is not in the log at the hour it was read' }), [
    ['Description', 'Raise war bonds.'], ['Full description', 'not recorded: This event is not in the log at the hour it was read'],
  ]);
  assert.deepEqual(tip({
    ...read,
    gaps: [
      { field: 'title', key: 'FROM.GetName', reason: 'targeted decision country has no exact occurrence-hour slot' },
      { field: 'description', key: 'ROOT.GetLeader', reason: 'reads more than a country name or date' },
    ],
  }), [
    ['Text not recorded (title)', 'FROM.GetName: targeted decision country has no exact occurrence-hour slot'],
    ['Text not recorded (description)', 'ROOT.GetLeader: reads more than a country name or date'],
    ['Description', 'Raise war bonds. Effects: +5% war support'],
  ], 'every gap the read row could not expand is listed with its field and reason once its row is read');
});

test('a decision cost read in one currency keeps it, partial, when the other currency was not read', () => {
  const start = day(3, 1);
  const occ = card([edge('decisionSelected', start, { days: 10, costs: pp('-5000000'), costsUnread: ['commandPower'] })]);
  const source = buildRowSourceFromHead('test:decisions-partial-cost', {
    branchId: '1', log: 'feed-events', columns: [], baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: '1', counts: [],
    rows: [{ hour: start, key: 'k1', ordinal: '0', cells: [], occurrence: occ }], facetTotals: [], missing: [], unavailable: [], warnings: [],
  }, undefined, start + 1)!;
  const model = decisionsProjection.project(createBoundedReadView({ hour: start + 1, rows: new Map([['main', source]]) }), { artwork: false, breaks: [] });
  assert.deepEqual(model.rows[0]?.cells.cost?.value, { kind: 'partial', value: '50', reason: 'Command power cost was not recorded' });
  const twice = card([edge('decisionSelected', start, { costs: pp('-5000000') }), edge('decisionSelected', start + 1)]);
  const both = buildRowSourceFromHead('test:decisions-partial-edges', {
    branchId: '1', log: 'feed-events', columns: [], baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: '1', counts: [],
    rows: [{ hour: start, key: 'k1', ordinal: '0', cells: [], occurrence: twice }], facetTotals: [], missing: [], unavailable: [], warnings: [],
  }, undefined, start + 2)!;
  const read = decisionsProjection.project(createBoundedReadView({ hour: start + 2, rows: new Map([['main', both]]) }), { artwork: false, breaks: [] });
  assert.deepEqual(read.rows[0]?.cells.cost?.value, { kind: 'partial', value: '50', reason: 'Part of the decision cost was not recorded' },
    'a selection whose cost was not read keeps the one that was, partial');
});

test('S3: a malformed decision row becomes a refused slot with its reason instead of disappearing or reporting no matches', () => {
  const start = day(3, 1);
  const hour = start + 10;
  const goodOcc = card([edge('decisionSelected', start, { days: 10 })]);
  const badOcc = { id: 'bad:dec', opened: true, edges: [] };
  const mixedHead = {
    branchId: '1',
    total: '2',
    offset: '0',
    live: '1',
    counts: [{ key: 'running', total: '2' }],
    rows: [
      { hour: start, key: goodOcc.id, observed: start, ordinal: '0', live: true, status: 'active', occurrence: goodOcc },
      { hour: start, key: 'bad:dec', observed: start, ordinal: '1', live: false, status: 'active', occurrence: badOcc },
    ],
    missing: [],
    nextChange: null,
    unavailable: [],
    warnings: [],
  };
  const mixedSource = buildRowSourceFromHead('test:decisions-mixed-bad', mixedHead, undefined, hour, undefined, decisionsProjection.validateTail)!;
  const mixedModel = decisionsProjection.project(
    createBoundedReadView({ hour, rows: new Map([['main', mixedSource]]) }),
    { artwork: false, breaks: [] },
  );
  assert.equal(mixedModel.rows.length, 1);
  assert.equal(mixedModel.rows[0]?.key, goodOcc.id);
  assert.equal(mixedModel.slots?.length, 2);
  assert.equal(mixedModel.slots?.[0]?.kind, 'data');
  assert.deepEqual(mixedModel.slots?.[1], { kind: 'refused', key: 'refused:1', reason: 'The occurrence card is malformed.' });
  assert.equal(mixedModel.titleSummary, '2 running');
  assert.equal(mixedModel.totalRows, 2);
  assert.equal(mixedModel.empty, undefined);

  const onlyBadHead = {
    ...mixedHead,
    total: '1',
    counts: [{ key: 'running', total: '1' }],
    rows: [{ hour: start, key: 'bad:dec', observed: start, ordinal: '0', live: false, status: 'active', occurrence: badOcc }],
  };
  const onlyBadSource = buildRowSourceFromHead('test:decisions-only-bad', onlyBadHead, undefined, hour, undefined, decisionsProjection.validateTail)!;
  const onlyBadModel = decisionsProjection.project(
    createBoundedReadView({ hour, rows: new Map([['main', onlyBadSource]]) }),
    { artwork: false, breaks: [] },
  );
  assert.equal(onlyBadModel.rows.length, 0);
  assert.deepEqual(onlyBadModel.slots, [{ kind: 'refused', key: 'refused:0', reason: 'The occurrence card is malformed.' }]);
  assert.equal(onlyBadModel.titleSummary, '1 running');
  assert.equal(onlyBadModel.totalRows, 1);
  assert.equal(onlyBadModel.empty, undefined, 'a tail with one malformed decision never claims no decisions matched');

  for (const declaration of [decisionsDeclaration, eventChoicesDeclaration, eventFeedDeclaration]) {
    const entry = declaration.sources({
      track: { source: { id: 's1', revision: 'r1', metadata: { playerCountryId: '9' } }, axisOffset: 0, selectedSlot: 7 },
      branch: { id: '1', firstHour: 0, lastHour: 200, gaps: [] },
      widget: { id: 'w1', kind: declaration.kind, countries: { kind: 'all' }, time: { mode: 'follow' } },
      options: { warsPeace: true },
      following: true,
      countries: { all: true, tags: [], filter: { passing: null, unrecorded: [], bounds: [] } },
      gateSpan: (firstHour: number, lastHour: number) => ({ firstHour, lastHour }),
    } as unknown as FamilyPlanContext<unknown>, { following: true, staticSpan: null }).entries[0]!;
    assert.equal(entry.read.shape === 'paged-rows' ? entry.read.validate : undefined, declaration.projection.validateTail, `${declaration.kind}: pagedTailEntry carries its product tail validator`);
  }
});
