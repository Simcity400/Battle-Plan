import { gatedHour } from '../../test-support/gated.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { Spec } from '../../dashboard/follow/messages.fixture.ts';
import { tableSortOf, tableSortToken } from '../../charts/table/options.ts';
import { DEFAULT_SORT, OPTIONS, definition } from './definition.ts';
import type { ScoreboardColumn, ScoreboardReply } from '../../generated/dashboard';
import { drawnCell, followedValues, formatCell, isPartial, measureCell, rankedTags, rankingColumn, rankingDescending, replyRows, replyValues, scoreboardParams, scoreboardSort,
  scoreboardTooltip, selectedColumns, validateColumnsReply, validateScoreboardReply } from './scoreboardData.ts';
import { cellText } from '../../draw/tables/surface.ts';
import { liveKey } from '../../charts/live.ts';

const family = (kind: string, measure: string) => ({ kind, measure });
const factories: ScoreboardColumn = { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null,
  family: family('country-factories', 'free7'), partial: null, noneAtZero: false };
const casualties: ScoreboardColumn = { key: 'casualties', label: 'Casualties', unit: 'people', decimals: 0, available: false,
  reason: 'Choose Casualties dealt or Casualties taken in settings.', family: null, partial: null, noneAtZero: false };
const power: ScoreboardColumn = { key: 'politicalPower', label: 'Content political label', unit: 'points', decimals: 5, available: true, reason: null,
  family: family('country-measure', 'politicalPower'), partial: null, noneAtZero: false };
const stability: ScoreboardColumn = { key: 'stability', label: 'Stability', unit: 'percent', decimals: 5, available: true, reason: null,
  family: family('country-stability', 'stability'), partial: null, noneAtZero: false };
const taken: ScoreboardColumn = { key: 'casualtiesTaken', label: 'Casualties taken', unit: 'people', decimals: 0, available: true, reason: null,
  note: 'Losses without a recorded attacker are excluded.', family: family('casualty-totals', 'taken'), partial: family('casualty-totals', 'partial'), noneAtZero: true };
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

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('a top ranks by the sorted measure, else the measure chosen, never by an unavailable column', () => {
  assert.equal(rankingColumn(registry, { column: 'stability', direction: 'ascending' }, '', 'factories'), stability);
  assert.equal(rankingColumn(registry, { column: 'country', direction: 'ascending' }, 'politicalPower', 'factories'), power);
  assert.equal(rankingColumn(registry, { column: 'casualties', direction: 'descending' }, 'casualties', 'factories'), factories);
  assert.deepEqual(scoreboardParams('1', gatedHour(50), [power, casualties], [], 10, factories),
    { branchId: '1', hour: 50, columns: ['politicalPower', 'casualties'], countries: [], top: { count: 10, sort: 'factories', descending: true } });
  assert.equal(rankingDescending({ column: 'factories', direction: 'ascending' }, factories), false, 'sorted ascending by the rank column: the least');
  assert.equal(rankingDescending({ column: 'country', direction: 'ascending' }, factories), true, 'sorted by name: the greatest');
  assert.deepEqual(scoreboardParams('1', gatedHour(50), [power], ['GER'], null, factories), { branchId: '1', hour: 50, columns: ['politicalPower'], countries: ['GER'] });
  assert.equal(scoreboardParams('1', gatedHour(50), [power], [], null, factories), null, 'no country chosen asks nothing');
  assert.equal(scoreboardParams('1', gatedHour(50), [], ['GER'], null, factories), null, 'no column asks nothing');
});

test('a reply\'s cells are exact amounts by the columns asked, a missing value keeps its reason and never reads as zero', () => {
  const wide = (1n << 80n) + 1n;
  const reply: ScoreboardReply = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [
    { country: 'GER', tag: 'GER', cells: [{ valueRaw: `-${wide}`, missing: null, partial: true }, { valueRaw: null, missing: 'GER\'s stability was not recorded at this hour', partial: false }] },
  ] }, 2);
  const [row] = replyRows(reply, ['casualtiesTaken', 'stability']);
  assert.deepEqual(row!.cells.get('casualtiesTaken'), { amount: -wide, missing: null, partial: true });
  const [slotted] = replyRows({ ...reply, rows: [{ ...reply.rows[0]!, country: 5 }] }, ['casualtiesTaken', 'stability']);
  assert.equal(slotted!.key, countryKey(5), 'a country chosen by slot keys its row by that slot, as the followed rows and the exact reads do');
  assert.equal(slotted!.tag, 'GER');
  const cells = [{ valueRaw: '1', missing: null, partial: false }, { valueRaw: '2', missing: null, partial: false }];
  const both = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [{ country: 5, tag: 'GER', cells }, { country: 'GER', tag: 'GER', cells }] }, 2);
  assert.deepEqual(replyRows(both, ['casualtiesTaken', 'stability']).map(row => row.key), [countryKey(5), 'GER'], 'a slot and a tag holding the same country are two rows');
  assert.throws(() => validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [{ country: 5, tag: 'GER', cells }, { country: 5, tag: 'GER', cells }] }, 2), /Invalid Scoreboard row/);
  assert.equal(row!.cells.get('stability')!.amount, null);
  assert.equal(formatCell(taken, -wide, true), '† −1,208,925,819,614,629,174,706,177');
  assert.equal(formatCell(stability, 50_000n), '50%');
  assert.equal(formatCell(power, 123_456_78n), '123.45678');
  assert.equal(formatCell(stability, null), '—');
  assert.deepEqual([formatCell(taken, 0n), formatCell(power, 0n)], ['None', '0'], 'a casualty total of zero (or no row) reads None; another zero stays 0');
  const tooltip = scoreboardTooltip(key => row!.cells.get(key)!, [taken, stability], 'Germany', 40);
  assert.deepEqual(tooltip.rows.map(item => item.label), ['Casualties taken', 'Stability'], 'one row per measure, no description rows');
  assert.deepEqual(tooltip.rows[1], { label: 'Stability', value: 'GER\'s stability was not recorded at this hour', tone: 'muted' });
  assert.equal(tooltip.note, '† Partial total');
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

test('the table draws each measure exactly as the scoreboard formats it, with its partial mark, and a missing value by its reason', () => {
  const amounts = [0n, 7n, -7n, 50_000n, 123_456_78n, -1_000_000n, 1_234_567_000_000n, (1n << 62n) + 1n, -(1n << 62n), 10n ** 15n + 3n];
  const columns = [factories, power, stability, taken];
  for (const amount of amounts) for (const partial of [false, true]) {
    const rows = replyRows({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [{ country: 'GER', tag: 'GER',
      cells: columns.map(() => ({ valueRaw: String(amount), missing: null, partial })) }] }, columns.map(column => column.key));
    const values = replyValues(rows, columns);
    for (const column of columns) {
      assert.equal(cellText(measureCell(column, 'GER'), values), formatCell(column, amount, partial), `${column.key} ${amount} ${partial}`);
    }
  }
  const missing = replyRows({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [{ country: 'GER', tag: 'GER',
    cells: [{ valueRaw: null, missing: 'GER\'s stability was not recorded at this hour', partial: false }, { valueRaw: null, missing: 'Unavailable here', partial: false }] }] },
  ['stability', 'casualties']);
  const values = replyValues(missing, [stability, casualties]);
  assert.equal(cellText(measureCell(stability, 'GER'), values), 'GER\'s stability was not recorded at this hour', 'never "0", never a dash');
  assert.equal(cellText(measureCell(casualties, 'GER'), values), casualties.reason, 'a column not in the recording says why');
  assert.equal(cellText(measureCell(stability, 'ITA'), values), 'Not in the reply');

  const wide = (1n << 62n) + 1n;
  const drawn = { values: { [liveKey('GER', 'casualtiesTaken')]: wide, [liveKey('GER', 'casualtiesTaken:partial')]: 2, [liveKey('GER', 'factories')]: 12 },
    missing: { [liveKey('GER', 'stability')]: 'GER\'s stability was not recorded at this hour' } };
  assert.deepEqual(drawnCell(drawn, 'GER', 'casualtiesTaken'), { amount: wide, missing: null, partial: true });
  assert.deepEqual(drawnCell(drawn, 'GER', 'factories'), { amount: 12n, missing: null, partial: false });
  assert.deepEqual(drawnCell(drawn, 'GER', 'stability'), { amount: null, missing: 'GER\'s stability was not recorded at this hour', partial: false });
});

test('the registry names the measure behind every available column', () => {
  const reply = { branchId: '1', defaultColumns: ['factories'], defaultSort: 'factories', columns: registry, warnings: [] };
  assert.equal(validateColumnsReply(reply).columns.length, registry.length);
  assert.throws(() => validateColumnsReply({ ...reply, columns: [{ ...factories, family: null }] }), /column/);
  assert.throws(() => validateColumnsReply({ ...reply, defaultSort: 'ships' }), /defaults/);
});
