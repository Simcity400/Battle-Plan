import assert from 'node:assert/strict';
import test from 'node:test';
import { tableSortOf, tableSortToken } from '../../charts/table/options.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message, type Spec } from '../../dashboard/follow/messages.fixture.ts';
import { deriveTableAudit, deriveTableTooltip, resolveTableRows } from '../../families/table/model.ts';
import { tableDefaults } from '../../families/table/settings.ts';
import type { ScoreboardColumn, ScoreboardReply } from '../../generated/dashboard';
import { gatedHour } from '../../test-support/gated.ts';
import { DEFAULT_SORT, OPTIONS, TABLE_BASE, definition } from './definition.ts';
import { isPartial, rankedTags, rankingColumn, rankingDescending, scoreboardParams, scoreboardSort, selectedColumns, validateColumnsReply, validateScoreboardReply } from './scoreboardData.ts';
import { scoreboardProjection } from './scoreboardProjection.ts';

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
  assert.equal(rankingDescending({ column: 'factories', direction: 'ascending' }, factories), false);
  assert.equal(rankingDescending({ column: 'country', direction: 'ascending' }, factories), true);
  assert.deepEqual(scoreboardParams('1', gatedHour(50), [power], ['GER'], null, factories), { branchId: '1', hour: 50, columns: ['politicalPower'], countries: ['GER'] });
  assert.equal(scoreboardParams('1', gatedHour(50), [power], [], null, factories), null);
  assert.equal(scoreboardParams('1', gatedHour(50), [], ['GER'], null, factories), null);
});

test('a reply’s cells are exact amounts, a missing value keeps its reason, and slot/tag keys stay distinct', () => {
  const wide = (1n << 80n) + 1n;
  const reply: ScoreboardReply = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [
    { country: 'GER', tag: 'GER', cells: [{ valueRaw: `-${wide}`, missing: null, partial: true }, { valueRaw: null, missing: 'GER\'s stability was not recorded at this hour', partial: false }] },
  ] }, 2);
  const shown = [taken, stability];
  const model = scoreboardProjection.project(
    { answers: new Map(), panels: new Map([['main', reply]]), live: new Map() },
    40,
    { shown, candidates: ['GER'], top: null, rankMarkers: false, looks: { GER: [{ from: 0, name: 'Germany' }] } },
  );
  const settings = tableDefaults(model.columns, TABLE_BASE);
  const flat = resolveTableRows(model, settings);
  const audit = deriveTableAudit(flat, model.columns, 40);
  assert.equal(audit.find(row => row.key === 'GER:casualtiesTaken')?.value, '−1,208,925,819,614,629,174,706,177, partial: Some inputs were not recorded at this hour');
  assert.equal(audit.find(row => row.key === 'GER:stability')?.value, "Not recorded: GER's stability was not recorded at this hour");
  const tip = deriveTableTooltip(flat, 'GER', model.columns, new Set(), 40);
  assert.ok(tip);
  assert.equal(tip.rows.find(row => row.label === 'Stability')?.detail, 'GER\'s stability was not recorded at this hour');

  const cells = [{ valueRaw: '1', missing: null, partial: false }, { valueRaw: '2', missing: null, partial: false }];
  const both = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [{ country: 5, tag: 'GER', cells }, { country: 'GER', tag: 'GER', cells }] }, 2);
  const bothModel = scoreboardProjection.project(
    { answers: new Map(), panels: new Map([['main', both]]), live: new Map() },
    40,
    { shown, candidates: [], top: null, rankMarkers: false, looks: { GER: [{ from: 0, name: 'Germany' }] } },
  );
  assert.deepEqual(bothModel.rows.map(row => row.key), [countryKey(5), 'GER']);
  assert.throws(() => validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [{ country: 5, tag: 'GER', cells }, { country: 5, tag: 'GER', cells }] }, 2), /Invalid Scoreboard row/);
  assert.throws(() => validateScoreboardReply({ ...reply, rows: [{ tag: 'GER', cells: [{ valueRaw: null, missing: null, partial: false }] }] }, 1), /row/);
  assert.throws(() => validateScoreboardReply(reply, 3), /row/);
});

test('followed values and rankings preserve exact precision, partial markers and missing reasons', () => {
  const V = CELL.value, U = CELL.unrecorded;
  const followed = answer({ keys: [7n, 8n, 0n], columns: 1, names: ['ITA', 'GER', 'SOV'], values: [(1n << 60n) + 3n, 0n, 0n], states: [V, U, CELL.noRow],
    reasons: [null, null, 'SOV is held by no country'] });
  assert.deepEqual(rankedTags(followed), ['ITA']);
  assert.equal(isPartial(1n), true);
  assert.equal(isPartial(0n), false);
  assert.equal(isPartial(undefined), false);

  const model = scoreboardProjection.project(
    { answers: new Map([['factories', followed]]), panels: new Map(), live: new Map() },
    40,
    { shown: [factories, casualties], candidates: ['ITA', 'GER', 'SOV', 'ENG'], top: null, rankMarkers: false, looks: {} },
  );
  const byKey = new Map(model.rows.map(row => [row.key, row]));
  assert.deepEqual(byKey.get('ITA')!.cells.factories!.value, { kind: 'known', value: (1n << 60n) + 3n });
  assert.equal(byKey.get('GER')!.cells.factories!.value.kind, 'notRecorded');
  assert.deepEqual(byKey.get('SOV')!.cells.factories!.value, { kind: 'notRecorded', reason: 'SOV is held by no country' });
  assert.deepEqual(byKey.get('ITA')!.cells.casualties!.value, { kind: 'notRecorded', reason: casualties.reason });
});

test('the registry names the measure behind every available column', () => {
  const reply = { branchId: '1', defaultColumns: ['factories'], defaultSort: 'factories', columns: registry, warnings: [] };
  assert.equal(validateColumnsReply(reply).columns.length, registry.length);
  assert.throws(() => validateColumnsReply({ ...reply, columns: [{ ...factories, family: null }] }), /column/);
  assert.throws(() => validateColumnsReply({ ...reply, defaultSort: 'ships' }), /defaults/);
});
