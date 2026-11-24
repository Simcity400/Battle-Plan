import assert from 'node:assert/strict';
import test from 'node:test';
import type { LogPage, LogRow } from '../../charts/log/logData';
import { eventDescription, eventTooltip, familyReveals, familyRows, familyShown, feedFacets, feedLogRow, feedRow, pageNotes, pagesNoted, rowMatches, validateFeedRow, validateSpan, withFamilyRows } from './eventFeedData.ts';
import type { FeedEvent } from './eventFeedData';
import type { EventFeedRow, FeedKindCode } from '../../generated/dashboard';

let serial = 0;
const row = (kind: EventFeedRow['kind'], hour: number, tags: string[], more: Partial<EventFeedRow> = {}): EventFeedRow => ({
  id: `native:1:${serial++}`, hour, readHour: hour, nameHour: hour, exact: true, category: 'warsPeace', kind, tags,
  participants: [tags[0] ?? null, tags[1] ?? null], item: null, amount: null, ...more });
const family = (kind: EventFeedRow['kind'], hour: number, tags: string[], category: EventFeedRow['category'] = 'warsPeace'): EventFeedRow => ({
  id: `${kind}:${hour}:${tags.join(',')}`, hour, readHour: hour, nameHour: hour, exact: true, category, kind, tags, item: null, amount: null });
const page = (key: string, rows: readonly LogRow<FeedEvent>[] | undefined, count = rows?.length ?? 3): LogPage<FeedEvent> => ({ key, start: 0, end: 0, count, rows });
const logRow = (event: EventFeedRow) => feedLogRow(event, event.id);
const hours = (pages: readonly LogPage<FeedEvent>[]) => pages.map(page => page.rows?.map(held => held.hour) ?? null);

test('a page row is its named row, validated, shown at its hour from its read, an icon the dashboard cannot draw dropped', () => {
  const event = row('warDeclared', 100, ['GER', 'POL'], { readHour: 101, cause: { how: 'a diplomatic action', within: null, conference: null, state: null,
    ally: null, reason: null, notes: [] } });
  const mapped = feedRow({ key: '7', hour: 101, cells: {}, fields: { row: event } });
  assert.deepEqual([mapped.key, mapped.hour, mapped.revealedHour, mapped.item.event], ['7', 100, 101, event]);
  const scripted = row('scriptEventFired', 100, ['GER', 'GER'], { category: 'politics',
    item: { token: 'news.1', name: 'News', icon: 'javascript:bad', description: null, options: [{ ordinal: 0, name: 'Fine' }] } });
  assert.equal(feedRow({ key: '8', hour: 100, cells: {}, fields: { row: scripted } }).item.item?.icon, null);
  assert.throws(() => feedRow({ key: '9', hour: 100, cells: {}, fields: { row: { ...event, participants: ['GER', 'ITA'] } } }), /malformed/,
    'roles disagree with the tags');
  assert.throws(() => feedRow({ key: '9', hour: 100, cells: {}, fields: { row: null } }), /malformed/, 'a row the feed does not show is never asked');
});

test('the kind facet takes the codes of the chosen categories, never a code no category holds', () => {
  const kinds: FeedKindCode[] = [{ kind: 'warDeclared', code: 0, category: 'warsPeace' }, { kind: 'scriptEventFired', code: 8, category: 'politics' },
    { kind: 'guaranteeGranted', code: 129, category: 'diplomacy' }];
  assert.deepEqual(feedFacets(kinds, new Set(['warsPeace', 'diplomacy'])), [{ name: 'kind', values: [0, 129], countries: [] }]);
  assert.equal(feedFacets(kinds, new Set()), null);
  assert.equal(feedFacets(kinds, new Set(['milestones'])), null, 'no kind of the log is a milestone: the log is not asked with an empty filter');
  assert.deepEqual(feedFacets(kinds, new Set(['milestones', 'politics'])), [{ name: 'kind', values: [8], countries: [] }]);
});

test('a row matches a chosen country through any country it names at its own hour, a vanished one at its last', () => {
  const chosen = (tag: string, hour: number) => tag === 'POL' && hour <= 150;
  assert.equal(rowMatches(row('warDeclared', 100, ['GER', 'POL']), chosen), true, 'the target');
  assert.equal(rowMatches(row('warJoined', 100, ['ITA', 'GER'], { cause: { how: null, within: null, conference: null, state: null, ally: 'POL', reason: null,
    notes: [] } }), chosen), true, 'a join\'s ally');
  assert.equal(rowMatches(family('ceasedToExist', 200, ['POL']), chosen), false);
  assert.equal(rowMatches({ ...family('ceasedToExist', 200, ['POL']), nameHour: 140 }, chosen), true, 'named at its last read with states');
});

test('other families\' rows fall among the loaded pages by hour, wait for a page they could belong to, and stop at the head', () => {
  const span = validateSpan({ branchId: '1', firstHour: 0, lastHour: 1000, kinds: [], unavailable: [], warnings: [],
    rows: [family('warStarted', 50, ['GER', 'POL']), family('factionJoined', 150, ['ITA'], 'diplomacy'), family('capitulated', 250, ['POL']),
      family('warStarted', 350, ['SOV', 'FIN'])] });
  const rows = familyRows(span, new Set(['warsPeace', 'diplomacy']), null, null);
  const first = page('0', [logRow(row('warDeclared', 100, ['GER', 'POL'])), logRow(row('annexed', 200, ['GER', 'AUS']))]);
  const second = page('1', [logRow(row('warEnded', 300, ['GER', 'POL'])), logRow(row('released', 400, ['GER', 'SLO']))]);
  assert.deepEqual(hours(withFamilyRows([first, second], rows, rows.length)), [[100, 200, 50, 150, 250], [300, 400, 350]]);
  assert.equal(withFamilyRows([first, second], rows, rows.length)[0].count, 5, 'counted by its rows');
  assert.deepEqual(hours(withFamilyRows([first, page('1', undefined, 500)], rows, rows.length)), [[100, 200, 50, 150], null],
    'a row after the page\'s last waits for the next page');
  assert.deepEqual(hours(withFamilyRows([first, second], rows, familyShown(rows, 260))), [[100, 200, 50, 150, 250], [300, 400]], 'the head has not read it');
  assert.deepEqual(hours(withFamilyRows([], rows, 2)), [[50, 150]], 'no feed row yet: a page of their own');
  assert.deepEqual(familyRows(span, new Set(['warsPeace']), 200, tag => tag === 'POL').map(held => held.hour), [250], 'category, range and country');
});

test("milestones alone are the span's rows, revealed by the hour each was read", () => {
  const span = validateSpan({ branchId: '1', firstHour: 0, lastHour: 1000, kinds: [], unavailable: [], warnings: [],
    rows: [{ ...family('projectFinished', 50, ['USA'], 'milestones'), readHour: 75 }, family('focusCompleted', 60, ['GER'], 'politics'),
      family('nukesIncreased', 70, ['USA'], 'milestones'), family('nukesIncreased', 80, ['USA'], 'milestones')] });
  const rows = familyRows(span, new Set(['milestones']), null, null);
  assert.deepEqual(familyReveals(rows), [70, 75, 80], 'ascending by the read that revealed each');
  assert.deepEqual(hours(withFamilyRows([], rows, familyShown(rows, 76))), [[50, 70]], 'no page: the rows read by the head, a page of their own');
  assert.deepEqual(hours(withFamilyRows([], rows, familyShown(rows, null))), [[50, 70, 80]]);
});

test('a row reads as a sentence naming its countries, how it happened and the ally a join fought beside', () => {
  const name = (tag: string) => ({ GER: 'Germany', POL: 'Poland', ITA: 'Italy' })[tag] ?? tag;
  const join = row('warJoined', 100, ['ITA', 'POL'], { cause: { how: 'a call to arms', within: null, conference: '7', state: null, ally: 'GER', reason: 'ally',
    notes: ['against every enemy of the ally\'s side'] } });
  assert.equal(eventDescription({ event: join, item: null }, name), 'Italy joined the war against Poland through a call to arms, alongside Germany');
  const tooltip = eventTooltip({ event: join, item: null }, name)[0];
  assert.deepEqual(tooltip.rows.map(held => held.label), ['Date', 'How', 'Hostility reason', 'Peace conference', 'against every enemy of the ally\'s side']);
  const unknown = row('annexed', 100, ['GER'], { participants: ['GER', null] });
  assert.equal(eventDescription({ event: unknown, item: null }, name), 'Germany annexed a country not identified');
  const taken = row('stateTransferred', 100, ['GER', 'POL'], { cause: { how: null, within: null, conference: null, state: 64, ally: null, reason: null, notes: [] } });
  assert.equal(eventDescription({ event: taken, item: null }, name), 'Germany took state 64 from Poland', 'a state not named is its number');
  assert.equal(eventTooltip({ event: row('warEnded', 100, ['GER', 'POL']), item: null }, name)[0].rows.at(-1)?.value, 'Not recorded in this recording',
    'a native row without its cause says so');
});

test('what pages say beside their rows is one line a note, its counts over every page held added, and a malformed page refused', () => {
  const refused = { kind: 'content', label: 'Event text', reason: 'The event database is unavailable', count: 2 };
  const first = pageNotes({ unavailable: [refused], warnings: ['Read at 10 lost'] });
  const second = pageNotes({ unavailable: [{ ...refused, count: 3 }, { kind: 'content', label: 'Pictures', reason: 'No art', count: null }], warnings: [] });
  assert.deepEqual(pagesNoted([...first, ...second]), ['Read at 10 lost', 'Event text: Not recorded — The event database is unavailable (5 counted)',
    'Pictures: Not recorded — No art']);
  assert.throws(() => pageNotes({ unavailable: [{ kind: 'content' }], warnings: [] }), /malformed/);
});

test('a nuclear strike names its bomb and target, totals only its complete parts, and a state-less impact says so', () => {
  const strike = { province: 13000, state: 1204, buildingLevels: 2.5, warSupport: -25, units: 1, manpowerLost: null, equipmentLost: null,
    notRecorded: ['equipment: writers not forwarded (1×)'] };
  const event = validateFeedRow({ ...row('nuclearStrike', 40, ['USA', 'JAP'], { readHour: 41, item: { token: 'nuclear_bomb', name: 'Atomic bomb', icon: null,
    description: null } }), strike });
  const at = { event, item: event.item };
  const name = (tag: string) => tag;
  assert.equal(eventDescription(at, name, () => 'Hiroshima'), 'USA dropped Atomic bomb on JAP');
  const [tip] = eventTooltip(at, name, () => 'Hiroshima');
  assert.deepEqual(tip.rows.slice(1, 4).map(item => item.value), ['Hiroshima', '13000', '2.5']);
  assert.ok(tip.rows.some(item => item.label === 'Manpower lost' && item.value === 'Not fully recorded'));
  assert.ok(!tip.rows.some(item => item.label === 'Cause'), 'a strike has no recorded cause to miss');
  const stateless = validateFeedRow({ ...row('nuclearStrike', 40, ['USA'], { participants: ['USA', null], item: null }), strike: { ...strike, state: null } });
  assert.equal(eventDescription({ event: stateless, item: null }, name), 'USA dropped a nuclear bomb of a type not known on a province without a state');
  assert.throws(() => validateFeedRow({ ...row('warDeclared', 40, ['USA', 'JAP']), strike }), /malformed/, 'only a strike carries one');
});

test('decision outcomes agree on the native Failed state, keep timeout cause, and surface named not-recorded and unsupported states', () => {
  const item = { token: 'mod_mission', name: 'Hold Sector', icon: null, description: null };
  const name = (tag: string) => tag;
  const timedFailed = validateFeedRow(row('decisionTimedOut', 100, ['GER'], {
    category: 'politics', participants: ['GER', null], item, decisionState: { kind: 'failed', observedHour: 101 },
  }));
  assert.equal(eventDescription({ event: timedFailed, item }, name), 'GER failed mission Hold Sector (reached the deadline)');
  const [failedTip] = eventTooltip({ event: timedFailed, item }, name);
  assert.ok(failedTip.rows.some(r => r.label === 'Mission state' && r.value === 'Failed'));
  assert.ok(failedTip.rows.some(r => r.label === 'Failure observed'));

  const completedFailed = validateFeedRow(row('decisionCompleted', 100, ['GER'], {
    category: 'politics', participants: ['GER', null], item, decisionState: { kind: 'failed', observedHour: 100 },
  }));
  assert.equal(eventDescription({ event: completedFailed, item }, name), 'GER failed mission Hold Sector');

  const unrecorded = validateFeedRow(row('decisionTimedOut', 100, ['GER'], {
    category: 'politics', participants: ['GER', null], item, decisionState: { kind: 'notRecorded', reason: 'mission failure state was not recorded' },
  }));
  assert.equal(eventDescription({ event: unrecorded, item }, name), 'GER reached the deadline of Hold Sector');
  assert.ok(eventTooltip({ event: unrecorded, item }, name)[0].rows.some(r => r.label === 'Mission failure state' && r.value === 'Not recorded — mission failure state was not recorded'));

  const notApplicable = validateFeedRow(row('decisionSelected', 100, ['GER'], {
    category: 'politics', participants: ['GER', null], item, decisionState: { kind: 'notApplicable', reason: 'decision has no mission lifecycle state' },
  }));
  assert.equal(eventDescription({ event: notApplicable, item }, name), 'GER selected decision Hold Sector');
  assert.ok(!eventTooltip({ event: notApplicable, item }, name)[0].rows.some(r => r.label === 'Mission failure state'));

  const unsupported = validateFeedRow(row('decisionTimedOut', 100, ['GER'], {
    category: 'politics', participants: ['GER', null], item, decisionState: { kind: 'unsupported', raw: 77 },
  }));
  assert.equal(eventDescription({ event: unsupported, item }, name), 'GER reached the deadline of Hold Sector (unsupported native state 77)');
  assert.throws(() => validateFeedRow(row('warDeclared', 100, ['GER', 'POL'], { decisionState: { kind: 'failed', observedHour: 100 } })), /malformed/);
});

