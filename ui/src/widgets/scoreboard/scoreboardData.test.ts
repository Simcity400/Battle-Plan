import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { Spec } from '../../dashboard/follow/messages.fixture.ts';
import { tableSortOf, tableSortToken } from '../../charts/table/options.ts';
import { DEFAULT_SORT, OPTIONS, definition } from './definition.ts';
import type { ScoreboardColumn, ScoreboardReply } from '../../generated/dashboard';
import { followedValues, formatCell, isPartial, rankedTags, rankingColumn, rankingDescending, replyRows, scoreboardParams, scoreboardSort, scoreboardTooltip,
  selectedColumns, validateColumnsReply, validateScoreboardReply } from './scoreboardData.ts';

const family = (kind: string, measure: string) => ({ kind, measure });
const factories: ScoreboardColumn = { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null,
  family: family('country-factories', 'free7'), partial: null };
const casualties: ScoreboardColumn = { key: 'casualties', label: 'Casualties', unit: 'people', decimals: 0, available: false,
  reason: 'Choose Casualties dealt or Casualties taken in settings.', family: null, partial: null };
const power: ScoreboardColumn = { key: 'politicalPower', label: 'Content political label', unit: 'points', decimals: 5, available: true, reason: null,
  family: family('country-measure', 'politicalPower'), partial: null };
const stability: ScoreboardColumn = { key: 'stability', label: 'Stability', unit: 'percent', decimals: 5, available: true, reason: null,
  family: family('country-stability', 'stability'), partial: null };
const taken: ScoreboardColumn = { key: 'casualtiesTaken', label: 'Casualties taken', unit: 'people', decimals: 0, available: true, reason: null,
  note: 'Losses without a recorded attacker are excluded.', family: family('casualty-totals', 'taken'), partial: family('casualty-totals', 'partial') };
const registry = [factories, casualties, power, stability, taken];
const answer = (spec: Spec) => parseFollowed(message(100, [{ id: 1, answer: spec }])).answers[0]!.answer;

test('the scoreboard follows playback over the registry defaults with the top ten, and its settings keep their meaning', () => {
  assert.equal(definition.defaultTime, 'follow');
  assert.equal(definition.defaultCountries.kind, 'group');
  assert.equal(OPTIONS.find(option => option.key === 'columns')?.kind, 'content-set');
  assert.deepEqual(DEFAULT_SORT, { column: '', direction: 'descending' });
  assert.deepEqual(selectedColumns(registry, '', ['factories', 'casualties']).map(column => column.key), ['factories', 'casualties']);
  assert.deepEqual(selectedColumns(registry, 'stability factories', ['factories']).map(column => column.key), ['stability', 'factories']);
  assert.deepEqual(scoreboardSort(registry, [casualties], { column: 'factories', direction: 'descending' }, 'factories'), { column: 'country', direction: 'descending' });
  assert.deepEqual(tableSortOf(tableSortToken({ column: 'politicalPower', direction: 'ascending' }), DEFAULT_SORT), { column: 'politicalPower', direction: 'ascending' });
});

test('a top ranks by the sorted measure, else the measure chosen, never by an unavailable column', () => {
  assert.equal(rankingColumn(registry, { column: 'stability', direction: 'ascending' }, '', 'factories'), stability);
  assert.equal(rankingColumn(registry, { column: 'country', direction: 'ascending' }, 'politicalPower', 'factories'), power);
  assert.equal(rankingColumn(registry, { column: 'casualties', direction: 'descending' }, 'casualties', 'factories'), factories);
  assert.deepEqual(scoreboardParams('1', 50, [power, casualties], [], 10, factories),
    { branchId: '1', hour: 50, columns: ['politicalPower', 'casualties'], tags: [], top: { count: 10, sort: 'factories', descending: true } });
  assert.equal(rankingDescending({ column: 'factories', direction: 'ascending' }, factories), false, 'sorted ascending by the rank column: the least');
  assert.equal(rankingDescending({ column: 'country', direction: 'ascending' }, factories), true, 'sorted by name: the greatest');
  assert.deepEqual(scoreboardParams('1', 50, [power], ['GER'], null, factories), { branchId: '1', hour: 50, columns: ['politicalPower'], tags: ['GER'] });
  assert.equal(scoreboardParams('1', 50, [power], [], null, factories), null, 'no country chosen asks nothing');
  assert.equal(scoreboardParams('1', 50, [], ['GER'], null, factories), null, 'no column asks nothing');
});

test('a reply\'s cells are exact amounts by the columns asked, a missing value keeps its reason and never reads as zero', () => {
  const wide = (1n << 80n) + 1n;
  const reply: ScoreboardReply = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [
    { tag: 'GER', cells: [{ valueRaw: `-${wide}`, missing: null, partial: true }, { valueRaw: null, missing: 'GER\'s stability was not recorded at this hour', partial: false }] },
  ] }, 2);
  const [row] = replyRows(reply, ['casualtiesTaken', 'stability']);
  assert.deepEqual(row!.cells.get('casualtiesTaken'), { amount: -wide, missing: null, partial: true });
  assert.equal(row!.cells.get('stability')!.amount, null);
  assert.equal(formatCell(taken, -wide, true), '† −1,208,925,819,614,629,174,706,177');
  assert.equal(formatCell(stability, 50_000n), '50%');
  assert.equal(formatCell(power, 123_456_78n), '123.45678');
  assert.equal(formatCell(stability, null), '—');
  const tooltip = scoreboardTooltip(key => row!.cells.get(key)!, [taken, stability], 'Germany', 40);
  assert.ok(tooltip.rows.some(item => item.label.includes('† Partial total')));
  assert.ok(tooltip.rows.some(item => item.label === 'GER\'s stability was not recorded at this hour'));
  assert.ok(tooltip.rows.some(item => item.label.includes('known attacker') || item.label.includes('recorded attacker')));
  assert.throws(() => validateScoreboardReply({ ...reply, rows: [{ tag: 'GER', cells: [{ valueRaw: null, missing: null, partial: false }] }] }, 1), /row/);
  assert.throws(() => validateScoreboardReply(reply, 3), /row/, 'a cell for every column asked');
});

test('followed values are read by tag exactly, a value not recorded is missing for its reason, and a rank names its countries in order', () => {
  const V = CELL.value, U = CELL.unrecorded;
  const followed = answer({ keys: [7n, 8n, 0n], columns: 1, names: ['ITA', 'GER', 'SOV'], values: [(1n << 60n) + 3n, 0n, 0n], states: [V, U, CELL.noRow],
    reasons: [null, null, 'SOV is held by no country'] });
  const values = followedValues(followed);
  assert.equal(values.get('ITA'), (1n << 60n) + 3n);
  assert.equal(values.get('GER'), undefined, 'a value not recorded is never zero');
  assert.equal(values.missing('GER'), 'Not recorded at this hour');
  assert.equal(values.missing('SOV'), 'SOV is held by no country');
  assert.equal(values.missing('ENG'), 'Not in the answer');
  assert.deepEqual(rankedTags(followed), ['ITA'], 'only countries with a value rank');
  assert.equal(followedValues(undefined).missing('ITA'), 'Not read yet');
  assert.equal(followedValues(answer({ notRecorded: 'No territory is recorded' })).missing('ITA'), 'No territory is recorded');
  assert.equal(isPartial(1n), true);
  assert.equal(isPartial(0n), false);
  assert.equal(isPartial(undefined), false, 'an unknown mark is not a subtotal');
});

test('the registry names the measure behind every available column', () => {
  const reply = { branchId: '1', defaultColumns: ['factories'], defaultSort: 'factories', columns: registry, warnings: [] };
  assert.equal(validateColumnsReply(reply).columns.length, registry.length);
  assert.throws(() => validateColumnsReply({ ...reply, columns: [{ ...factories, family: null }] }), /column/);
  assert.throws(() => validateColumnsReply({ ...reply, defaultSort: 'ships' }), /defaults/);
});
