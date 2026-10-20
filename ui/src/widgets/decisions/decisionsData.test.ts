import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import type { EventFeedItem, EventFeedKind, EventFeedRow, ScriptCost, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';
import { recordingBreaks, validateScriptPage } from '../event-feed/scriptOccurrences.ts';
import { OPTIONS, definition } from './definition.ts';
import { activeCount, costText, daysLeft, decisionAt, decisionDescription, decisionDiagnostics, decisionJumps, decisionName, decisionTooltip,
  lifecycleLine, prepareDecisions, STATUS_CHOICES, statusLabel, statusTone } from './decisionsData.ts';
import type { DecisionFilter } from './decisionsData';

const day = (month: number, date: number, year = 1939) => engineHour({ year, month, day: date, hour: 1 })!;
const START = day(1, 1), END = day(12, 31);
let serial = 0;
const edge = (kind: EventFeedKind, hour: number, extra: Partial<ScriptEdge> = {}, more: Partial<EventFeedRow> = {},
  participants: [string | null, string | null] = ['mod | taker', null]): ScriptEdge => ({
  row: { id: `native:2:${serial++}`, hour, readHour: hour, nameHour: hour, exact: true, category: 'politics', kind,
    tags: participants.filter((tag): tag is string => tag !== null), participants, item: 0, amount: null, ...more },
  option: null, lifecycle: null, days: null, costs: null, ...extra });
const card = (edges: ScriptEdge[], opened = true): ScriptOccurrence => ({ id: edges[0].row.id, opened, edges });
const item: EventFeedItem = { token: 'mod_decision', name: 'Modded decision', icon: null, description: 'Authored effects.', idea: null };
const page = (occurrences: ScriptOccurrence[], items: EventFeedItem[] = [item]) => validateScriptPage({
  reply: { branchId: '1', firstHour: START, lastHour: END, occurrences, items, unavailable: [], warnings: [] }, nextCursor: null }).reply;
const all: DecisionFilter = { country: () => true, allCountries: true, firstHour: null };
const every = new Set<string>(STATUS_CHOICES.map(status => status.value));
const at = (occurrences: ScriptOccurrence[], hour: number, statuses: ReadonlySet<string> = every, breaks = [] as ReturnType<typeof recordingBreaks>) =>
  prepareDecisions(page(occurrences), all).flatMap(row => { const shown = decisionAt(row, hour, breaks, statuses); return shown ? [shown.item] : []; });
const one = (edges: ScriptEdge[], hour = END, opened = true) => at([card(edges, opened)], hour)[0];
const name = (tag: string) => tag.replace('mod | ', '');
const pp = (amount: string): ScriptCost[] => [{ currency: 'politicalPower', amount }];

test('each lifecycle path has its own status and line', () => {
  // A timed decision counting down, then removed by the game at its end.
  const timed = [edge('decisionSelected', day(3, 1), { days: 70 })];
  const running = one(timed, day(3, 1) + 58 * 24);
  assert.deepEqual([statusLabel(running), statusTone(running), lifecycleLine(running), daysLeft(running)], ['Active', 'accent', '70-day timer · 12 days left', 12]);
  assert.equal(lifecycleLine(one(timed, day(3, 1) + 69 * 24 + 1)), '70-day timer · 1 day left');
  const removed = one([...timed, edge('decisionRemoved', day(5, 10))]);
  assert.deepEqual([statusLabel(removed), statusTone(removed), lifecycleLine(removed)], ['Removed', 'neutral', 'Removed by the game']);
  // A mission met, and one that timed out.
  const mission = [edge('decisionActivated', day(4, 1), { days: 20 })];
  const met = one([...mission, edge('decisionCompletionEffect', day(4, 12)), edge('decisionCompleted', day(4, 12))]);
  assert.deepEqual([statusLabel(met), statusTone(met), lifecycleLine(met)], ['Completed', 'positive', 'Activated 1 Apr 1939 → completed 12 Apr 1939']);
  const late = one([...mission, edge('decisionTimeoutEffect', day(4, 21)), edge('decisionTimedOut', day(4, 21))]);
  assert.deepEqual([statusLabel(late), statusTone(late), lifecycleLine(late)], ['Timed out', 'neutral', 'Timed out after 20 days']);
  const cancelled = one([edge('decisionSelected', day(6, 1), { days: 30 }), edge('decisionCancelled', day(6, 15))]);
  assert.deepEqual([statusLabel(cancelled), lifecycleLine(cancelled)], ['Cancelled', 'Cancelled after 14 days']);
  // An instant decision is taken, never "completed" without a completion.
  const instant = one([edge('decisionSelected', day(7, 4)), edge('decisionCompletionEffect', day(7, 4))]);
  assert.deepEqual([statusLabel(instant), statusTone(instant), lifecycleLine(instant)], ['Taken', 'neutral', 'Selected 4 Jul 1939']);
  // A completion whose selection was not recorded is its own row.
  const alone = one([edge('decisionCompleted', day(8, 1))], END, false);
  assert.deepEqual([statusLabel(alone), lifecycleLine(alone), alone.start], ['Completed', 'Completed 1 Aug 1939', null]);
  const tooltip = decisionTooltip(met, name)[0];
  assert.deepEqual(tooltip.rows.map(row => row.label), ['Country', 'Status', 'Activated', 'Completion effects ran', 'Completed', 'Timer', 'Cost',
    'Content identity', 'Authored effects.']);
  assert.deepEqual(decisionJumps(met), [day(4, 1), day(4, 12)]);
});

test('one decision taken several times is several rows, ordered by the date selected and updated in place', () => {
  const first = card([edge('decisionSelected', day(2, 1), { days: 10 }), edge('decisionRemoved', day(2, 11))]);
  const second = card([edge('decisionSelected', day(3, 1), { days: 10 }), edge('decisionDurationChanged', day(3, 5), { days: 30 })]);
  const rows = prepareDecisions(page([first, second]), all);
  assert.deepEqual(rows.map(row => row.hour), [day(3, 1), day(2, 1)]);
  const before = decisionAt(rows[0], day(3, 4), [], every)!;
  const after = decisionAt(rows[0], day(3, 6), [], every)!;
  assert.equal(before.key, after.key);
  assert.equal(before.hour, after.hour);
  assert.equal(lifecycleLine(before.item), '10-day timer · 7 days left');
  assert.equal(lifecycleLine(after.item), '30-day timer · 29 days left');
  assert.equal(activeCount([before, decisionAt(rows[1], day(3, 6), [], every)!]), 1);
  // Seeking back hides rows not yet selected.
  assert.equal(decisionAt(rows[0], day(2, 20), [], every), null);
});

test('status chips filter rows; Taken goes with Completed and an unknown end with Active', () => {
  const data = [card([edge('decisionSelected', day(3, 1))]), card([edge('decisionSelected', day(3, 2), { days: 5 }), edge('decisionCancelled', day(3, 4))]),
    card([edge('decisionActivated', day(3, 3), { days: 50 })])];
  assert.deepEqual(at(data, END).map(row => row.status), ['active', 'cancelled', 'taken']);
  assert.deepEqual(at(data, END, new Set(['completed'])).map(row => row.status), ['taken']);
  assert.deepEqual(at(data, END, new Set(['active', 'cancelled'])).map(row => row.status), ['active', 'cancelled']);
  const breaks = recordingBreaks({ gaps: [{ firstHour: day(4, 10), lastHour: day(4, 12) }] });
  const [paused] = at(data, day(5, 1), new Set(['active']), breaks);
  assert.deepEqual([paused.status, statusLabel(paused), statusTone(paused), daysLeft(paused)], ['unknown', 'Not recorded', 'neutral', null]);
  assert.equal(lifecycleLine(paused), 'Activated 3 Mar 1939 · no end recorded by 9 Apr 1939');
  assert.equal(decisionTooltip(paused, name)[0].rows.find(row => row.label === 'Recording paused')?.tone, 'muted');
  assert.equal(at(data, day(4, 1), new Set(['active']), breaks)[0].status, 'active');
  const options = resolveOptions(OPTIONS, {});
  assert.deepEqual(STATUS_CHOICES.map(status => options[status.value]), [true, true, true, true, true]);
  assert.deepEqual([definition.defaultCountries, definition.countries, definition.category], [{ kind: 'selected' }, { max: Number.MAX_SAFE_INTEGER, all: true }, 'Politics']);
});

test('a cost shows only where a payment is proven, as political power spending writes it', () => {
  assert.equal(costText(one([edge('decisionSelected', day(3, 1))])), null);
  assert.equal(costText(one([edge('decisionSelected', day(3, 1), { costs: pp('-5000000') })])), '−50 PP');
  assert.equal(costText(one([edge('decisionSelected', day(3, 1), { costs: [...pp('-7500000'), { currency: 'commandPower', amount: '-1000000' }] })])), '−75 PP, −10 CP');
  assert.equal(costText(one([edge('decisionSelected', day(3, 1), { costs: pp('2500000') })])), '+25 PP');
  assert.equal(costText(one([edge('decisionSelected', day(3, 1), { costs: [] })])), 'None');
  const unproven = one([edge('decisionSelected', day(3, 1))]);
  assert.deepEqual(decisionTooltip(unproven, name)[0].rows.find(row => row.label === 'Cost'), { label: 'Cost', value: 'Not recorded', tone: 'muted' });
  assert.match(decisionDescription(unproven, name), /^taker, Modded decision, taken, Selected 1 Mar 1939, cost not recorded$/);
});

test('country narrowing matches any recorded country of a row at its own hour; missing names are counted', () => {
  const targeted = card([edge('decisionSelected', day(3, 1), {}, {}, ['mod | taker', 'mod | target'])]);
  const other = card([edge('decisionSelected', day(3, 2), {}, {}, ['mod | other', null])]);
  const narrowed = (tag: string): DecisionFilter => ({ ...all, allCountries: false, country: candidate => candidate === tag });
  assert.deepEqual(prepareDecisions(page([targeted, other]), narrowed('mod | target')).map(row => row.item.card.country), ['mod | taker']);
  assert.equal(prepareDecisions(page([targeted, other]), narrowed('mod | target'))[0].item.card.target, 'mod | target');
  assert.deepEqual(prepareDecisions(page([targeted, other]), narrowed('mod | other')).map(row => row.item.card.country), ['mod | other']);
  assert.deepEqual(prepareDecisions(page([targeted, other]), { ...all, firstHour: day(3, 2) }).map(row => row.item.card.country), ['mod | other']);
  const unnamed = page([card([edge('decisionSelected', day(3, 1))]), card([edge('decisionSelected', day(3, 2))])],
    [{ ...item, name: null }]);
  assert.equal(decisionName(prepareDecisions(unnamed, all)[0].item.card), 'Decision name unavailable');
  assert.deepEqual(decisionDiagnostics([unnamed]), ['Decision name unavailable: mod_decision (2 counted)']);
});
