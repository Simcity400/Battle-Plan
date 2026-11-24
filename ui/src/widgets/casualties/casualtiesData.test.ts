import assert from 'node:assert/strict';
import test from 'node:test';
import type { SeriesStateReply } from '../../generated/dashboard';
import { answerHeld, answerReasons, casualtyRaceEntries, casualtyRatio, casualtyRows, casualtyTooltip, casualtyUnknownNote, formatCasualties, rankCasualties, stateHeld, heldCasualties, retainedLive } from './casualtiesData.ts';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { HeldMeasures } from './casualtiesData';

const state = (measure: string, values: Record<string, string | null>): SeriesStateReply => ({ branchId: '1', measure, decimals: 0,
  states: Object.entries(values).map(([tag, raw]) => ({ country: tag, hour: 10, drop: null, missing: raw === null ? 'Not recorded' : null,
    run: raw === null ? null : { valueRaw: raw } as never })) });

const held = (): HeldMeasures => ({
  dealt: stateHeld(state('dealt', { AAA: '50', BBB: '9', CCC: null })),
  taken: stateHeld(state('taken', { AAA: '20', BBB: '40', CCC: '7' })),
  unknown: stateHeld(state('unknown', { AAA: '5', BBB: null, CCC: '1' })),
  partial: stateHeld(state('partial', { AAA: '0', BBB: '1', CCC: '0' })),
});

test('unknown losses add to taken only when known, and a lost lifetime marks a subtotal without a ratio', () => {
  const rows = casualtyRows(['AAA', 'BBB', 'CCC', 'DDD'], held(), 10, true);
  assert.deepEqual(rows.map(row => [row.tag, row.taken, row.atLeast]), [['AAA', 25n, false], ['BBB', null, true], ['CCC', 8n, false]],
    'a country with no casualty row is left out; an unknown total leaves taken unknown, never the known part alone');
  assert.equal(casualtyRows(['BBB'], held(), 10, false)[0].taken, 40n);
  assert.equal(casualtyRatio(rows[1]), '—');
  assert.equal(formatCasualties(rows[1].taken, rows[1].atLeast), '—');
  assert.equal(formatCasualties(25n, true), '† 25');
});

test('the top countries rank by dealt plus known taken, rows not known last', () => {
  const rows = casualtyRows(['CCC', 'BBB', 'AAA'], held(), 10, true);
  assert.deepEqual(rankCasualties(rows).map(row => row.tag), ['AAA', 'BBB', 'CCC']);
  assert.deepEqual(rankCasualties(rows, 1).map(row => row.tag), ['AAA']);
});

test('the race is of the ranked rows, ordered by known taken and drawn at taken with unknown attackers when asked', () => {
  const rows = rankCasualties(casualtyRows(['CCC', 'BBB', 'AAA'], held(), 10, true), 2);
  assert.deepEqual(rows.map(row => row.tag), ['AAA', 'BBB'], 'membership is by dealt plus known taken');
  assert.deepEqual(casualtyRaceEntries(rows).map(entry => [entry.tag, entry.value, entry.label]), [['AAA', 25, '25']],
    'a row whose taken with unknown attackers is not known is not raced');
  const known = rankCasualties(casualtyRows(['CCC', 'BBB', 'AAA'], held(), 10, false), 3);
  assert.deepEqual(casualtyRaceEntries(known).map(entry => [entry.tag, entry.value, entry.label]), [['BBB', 40, '† 40'], ['AAA', 20, '20'], ['CCC', 7, '7']],
    'ordered by known taken, not by the membership rank');
});

test('a top N ranks no country known to total zero; every country keeps it, and an unknown total is never a zero', () => {
  const values = { AAA: '0', BBB: '3', CCC: null, DDD: '0' };
  const measures: HeldMeasures = { dealt: stateHeld(state('dealt', values)), taken: stateHeld(state('taken', { AAA: '0', BBB: '0', CCC: '0', DDD: '0' })),
    unknown: stateHeld(state('unknown', { AAA: '0', BBB: '0', CCC: '0', DDD: '0' })), partial: null };
  const rows = casualtyRows(['AAA', 'BBB', 'CCC', 'DDD'], measures, 10, false);
  assert.deepEqual(rankCasualties(rows, 10, true).map(row => row.tag), ['BBB', 'CCC'], 'no 0/0 rows pad the top in tag order');
  assert.deepEqual(rankCasualties(rows, 10).map(row => row.tag), ['BBB', 'AAA', 'DDD', 'CCC']);
});

test('review 8 2: a ranking awaiting its measures is loading, an answered empty one is empty, and a changed membership keeps the last complete result', () => {
  const first = heldCasualties<string>(null, [undefined, undefined], ['GER', 'SOV'], true, null);
  assert.deepEqual([first.awaiting, first.held], [true, null], 'a nonempty ranking whose measures are pending is loading, not "No casualties yet"');
  assert.equal(heldCasualties<string>(null, [undefined, undefined], [], true, null).awaiting, false, 'an answered empty ranking is empty');
  const complete = heldCasualties<string>(null, ['a', 'b'], ['GER', 'SOV'], true, null);
  const moved = heldCasualties<string>(complete.held, [undefined, undefined], ['GER', 'ITA'], true, null);
  assert.deepEqual([moved.lives, moved.members, moved.awaiting], [['a', 'b'], ['GER', 'SOV'], false], 'a new member keeps the last complete result until its measures land');
  assert.equal(heldCasualties<string>(complete.held, [undefined, undefined], ['GER'], true, 'Not recorded').held, null, 'a refusal drops the held result');
});

test('review 9: a retained reader keeps the last complete values when its installation no longer holds them', () => {
  let answer: string | undefined = 'complete';
  const live = { read: () => ({ answer, version: answer ? 1 : 2 }), subscribe: () => () => {} };
  const held = retainedLive(live);
  assert.equal(held.read().answer, 'complete');
  answer = undefined;
  assert.deepEqual(held.read(), { answer: 'complete', version: 1 }, 'a rank-only install does not lose the previous measures');
  assert.equal(retainedLive(live), held, 'one retained reader per reader, so members and values stay paired');
});

test('review 10: on the render that changes membership, answers still from the previous subscriptions are not saved as complete', () => {
  const before = heldCasualties<string>(null, ['a', 'b'], ['GER', 'SOV'], true, null);
  const mixed = heldCasualties<string>(before.held, ['a', 'b'], ['GER', 'ITA'], true, null, false);
  assert.deepEqual([mixed.members, mixed.lives, mixed.held], [['GER', 'SOV'], ['a', 'b'], before.held], 'the previous membership and values stay together');
});

test('explicitly requested unread keyed rows keep their row and truncation reason through to the tooltip', () => {
  const trunc = 'Casualty totals read only in part at this hour (1 rows not read)';
  const followAnswer = (vals: [string, bigint, number, string | null][]) => parseFollowed(message(128, [{ id: 1, answer: {
    keys: vals.map((_, i) => BigInt(i + 1)), columns: 1, values: vals.map(([, v]) => v),
    states: vals.map(([, , s]) => s), names: vals.map(([t]) => t), reasons: vals.map(([, , , r]) => r),
  } }])).answers[0]!.answer;
  const dealt = followAnswer([['GER', 50n, CELL.value, null], ['ITA', 0n, CELL.noRow, trunc], ['AUS', 0n, CELL.noRow, 'AUS is held by no country']]);
  const taken = followAnswer([['GER', 20n, CELL.value, null], ['ITA', 0n, CELL.noRow, trunc], ['AUS', 0n, CELL.noRow, 'AUS is held by no country']]);
  const asked = ['GER', 'ITA', 'AUS'];
  const measures: HeldMeasures = { dealt: answerHeld(dealt, asked), taken: answerHeld(taken, asked), unknown: null, partial: null };
  const reasons = answerReasons([dealt, taken], asked);
  const rows = casualtyRows(asked, measures, 128, false, reasons);
  assert.deepEqual(rows.map(row => [row.tag, row.dealt, row.taken, row.reason ?? null]), [
    ['GER', 50n, 20n, null],
    ['ITA', null, null, trunc],
  ]);
  assert.equal(casualtyTooltip(rows[1]!, 'Italy', 'Unknown attacker').note, trunc);
  assert.deepEqual(casualtyRaceEntries(rows).map(entry => [entry.tag, entry.value]), [['GER', 20]]);
  assert.equal(
    casualtyUnknownNote(rows, tag => tag === 'ITA' ? 'Italy' : tag),
    `Losses taken are unknown for Italy: ${trunc}.`,
  );
});
