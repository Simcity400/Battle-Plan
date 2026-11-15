import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChartSummary } from '../chartSummary';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { CountryGroupReply, FactionRostersReply, SeriesStateReply } from '../../generated/dashboard';
import type { FactionLines } from '../../dashboard/territoryReplies';
import { answerShape, answerValues, factionEntries, factionNote, factionSummaries, factionMembersAt, factionMemberSpans, fixedRosters, GROUP_ID, groupTotal, membersPushed,
  rosterRows, rosterTags, stateValues } from './factionTotalsData.ts';

const incomplete = 'Its roster was not read whole at this hour';
const rosters: FactionRostersReply = { missing: null, warnings: [], factions: [
  { id: '1', members: ['GER', 'ITA'], missing: null },
  { id: '2', members: ['SOV'], missing: null },
  { id: '3', members: ['ENG'], missing: null },
  { id: '4', members: [], missing: null },
  { id: '5', members: ['JAP'], missing: incomplete },
] };

const answer = (rows: [string, bigint, number?][]) => parseFollowed(message(10, [{ id: 1, answer: { keys: rows.map((_, at) => BigInt(at + 1)), columns: 1,
  values: rows.map(([, value]) => value), states: rows.map(([, , state]) => state ?? CELL.value), names: rows.map(([tag]) => tag) } }])).answers[0]!.answer;

test('a faction total is its members’ followed values added; one member not known leaves it out, never smaller', () => {
  const values = answerValues(answer([['GER', 30n], ['ITA', 10n], ['SOV', 70n, CELL.unrecorded], ['ENG', 55n], ['JAP', 9n]]), rosterTags(rosters), 0);
  assert.deepEqual([...values], [['GER', 30], ['ITA', 10], ['ENG', 55], ['JAP', 9]]);
  const { entries, unknown } = factionEntries(rosters, values, 10, 7, String);
  assert.deepEqual(entries.map(entry => [entry.tag, entry.value, entry.hour]), [['3', 55, 7], ['1', 40, 7]]);
  assert.equal(unknown, 2, 'a member without a value and an incomplete roster');
  assert.equal(factionEntries(rosters, values, 1, 7, String).entries.length, 1);
  assert.equal(factionNote(rosters, '5', false), incomplete);
});

test('a follow acknowledged before its first answer leaves the race on the members’ asked states, never empty', () => {
  assert.equal(membersPushed('followed', undefined), false, 'paused, the answer waits for the head to take its step');
  const asked: SeriesStateReply = { decimals: 0, states: [['GER', '30'], ['ITA', '10'], ['SOV', '70'], ['ENG', '55']].map(([tag, valueRaw]) =>
    ({ tag, part: null, hour: 7, run: { valueRaw }, drop: null, missing: null })) } as unknown as SeriesStateReply;
  const shown = factionEntries(rosters, stateValues(asked, 7), 10, 7, String).entries;
  assert.deepEqual(shown.map(entry => [entry.tag, entry.value]), [['2', 70], ['3', 55], ['1', 40]]);
  const landed = answer([['GER', 30n]]);
  assert.equal(membersPushed('followed', landed), true);
  assert.equal(membersPushed('pending', landed), false);
  assert.equal(membersPushed('off', landed), false);
  assert.notEqual(answerShape(undefined), answerShape(landed), 'the first answer landing renders the race');
  assert.notEqual(answerShape(landed), answerShape({ kind: 'notRecorded', reason: 'not read' }), 'a refusal renders its reason');
  assert.equal(answerShape(landed), answerShape(answer([['GER', 31n]])), 'a value moving renders nothing');
});

test('the race renders for its rosters, never for a number', () => {
  assert.deepEqual(rosterTags(rosters), ['ENG', 'GER', 'ITA', 'JAP', 'SOV']);
  const moved = { ...rosters, factions: rosters.factions.map(faction => faction.id === '2' ? { ...faction, members: ['SOV', 'MON'] } : faction) };
  assert.notEqual(rosterRows(moved), rosterRows(rosters));
  assert.equal(rosterRows({ ...rosters }), rosterRows(rosters));
});

test('the faction lines drawn are the greatest at the end of the view', () => {
  const summary = (last: number | null): ChartSummary => ({ from: 0, to: 10, width: 2, buckets: [last === null ? null : { firstHour: 0, lastHour: 10, firstValue: last,
    lastValue: last, lastValueHour: 10, minHour: 0, minValue: last, maxHour: 10, maxValue: last, firstBreak: true, hasBreak: false }] });
  const lines: FactionLines = { branchId: '1', measure: 'free7', decimals: 0, bounds: true, warnings: [],
    lines: [{ id: '1', summary: summary(10), missing: [] }, { id: '2', summary: summary(30), missing: [] }, { id: '3', summary: summary(null), missing: [] }] };
  assert.deepEqual([...factionSummaries(lines, 2).keys()], ['2', '1']);
  assert.match(factionNote(undefined, '1', true)!, /bounds/);
});

test('a roster fixed at the view’s end, or of anyone ever a member, is read from the recorded memberships', () => {
  const run = (tag: string, firstHour: number, lastHour: number) => ({ tag, firstHour, lastHour });
  const faction = (id: string, members: ReturnType<typeof run>[], incomplete: { firstHour: number; lastHour: number }[] = []) =>
    ({ id, firstHour: 0, lastHour: 500, members, incomplete }) as unknown as CountryGroupReply['factions'][number];
  const groups = { factions: [faction('1', [run('GER', 0, 500), run('ITA', 20, 40), run('ITA', 60, 90), run('HUN', 150, 300)]),
    faction('2', [run('ENG', 0, 500)], [{ firstHour: 30, lastHour: 35 }])], unobserved: { factions: [] } } as unknown as CountryGroupReply;
  const end = fixedRosters(groups, 'end', 10, 100);
  assert.deepEqual(end.factions.map(item => [item.id, item.members, item.missing]), [['1', ['GER'], null], ['2', ['ENG'], null]]);
  const ever = fixedRosters(groups, 'ever', 10, 100);
  assert.deepEqual(ever.factions.map(item => [item.id, item.members]), [['1', ['GER', 'ITA']], ['2', ['ENG']]]);
  assert.match(ever.factions[1].missing!, /not read whole/, 'a roster incomplete within the view is not known, never smaller');
  const unread = fixedRosters({ ...groups, unobserved: { factions: [{ firstHour: 100, lastHour: 120 }] } } as unknown as CountryGroupReply, 'end', 10, 100);
  assert.deepEqual([unread.factions, typeof unread.missing], [[], 'string']);
});
test('a group shown as one total sums its members at each hour they are members, and is not known where they were not read', () => {
  const hourly = groupTotal({ tags: ['GER', 'ITA'], membership: new Map([['GER', [{ tag: 'GER', firstHour: 0, lastHour: 99 }]], ['ITA', [{ tag: 'ITA', firstHour: 50, lastHour: 60 }]]]),
    unobserved: [{ firstHour: 70, lastHour: 80 }] }, 'Axis');
  assert.deepEqual(hourly.members, [{ tag: 'GER', spans: [[0, 99]] }, { tag: 'ITA', spans: [[50, 60]] }]);
  assert.deepEqual(hourly.rosterAt(10).factions, [{ id: GROUP_ID, members: ['GER'], missing: null }]);
  assert.deepEqual(hourly.rosterAt(55).factions[0].members, ['GER', 'ITA']);
  assert.ok(hourly.rosterAt(75).factions[0].missing, 'members not read: the total is not known, never smaller');
  const values = new Map([['GER', 5], ['ITA', 3]]);
  assert.deepEqual(factionEntries(hourly.rosterAt(55), values, 1, 55, String).entries.map(entry => [entry.tag, entry.value]), [[GROUP_ID, 8]]);
  assert.deepEqual(factionEntries(hourly.rosterAt(75), values, 1, 75, String), { entries: [], unknown: 1 });
  const ever = groupTotal({ tags: ['GER', 'ITA'], membership: null, unobserved: [] }, 'Players');
  assert.deepEqual(ever.members.map(member => member.spans), [null, null], 'counted at the end or ever: members over the whole view');
  assert.deepEqual(ever.rosterAt(500).factions[0].members, ['GER', 'ITA']);
});

test('a faction’s members are drawn over the hours each was one, and listed at an hour as members then', () => {
  const faction = { members: [{ tag: 'ITA', firstHour: 50, lastHour: 60 }, { tag: 'GER', firstHour: 0, lastHour: 40 }, { tag: 'ITA', firstHour: 61, lastHour: 70 },
    { tag: 'GER', firstHour: 45, lastHour: 99 }] };
  assert.deepEqual([...factionMemberSpans(faction)], [['GER', [{ firstHour: 0, lastHour: 40 }, { firstHour: 45, lastHour: 99 }]], ['ITA', [{ firstHour: 50, lastHour: 70 }]]]);
  assert.deepEqual(factionMembersAt(faction, 42), []);
  assert.deepEqual(factionMembersAt(faction, 55), ['GER', 'ITA']);
});
