import test from 'node:test';
import assert from 'node:assert/strict';
import type { ScoreboardColumn, ScoreboardReply, ScoreboardSeries } from '../../generated/dashboard';
import { rankChanges, sortRows } from '../../charts/table/logic.ts';
import { tableSortOf, tableSortToken } from '../../charts/table/options.ts';
import { rankingSort, scoreboardModel, scoreboardRequest, scoreboardRows, scoreboardSort, scoreboardTooltip, selectedColumns, formatCell, validateScoreboardReply } from './scoreboardData.ts';
import { DEFAULT_SORT, OPTIONS, definition } from './definition.ts';

const factories: ScoreboardColumn = { key: 'factories', label: 'Content factory label', unit: 'count', decimals: 0, available: true, reason: null };
const army: ScoreboardColumn = { key: 'armySize', label: 'Army size', unit: 'divisions', decimals: 0, available: false, reason: 'Not available yet: the Army size shared measure is not on main.' };
const power: ScoreboardColumn = { key: 'politicalPower', label: 'Content political label', unit: 'points', decimals: 5, available: true, reason: null };
const stability: ScoreboardColumn = { key: 'stability', label: 'Content stability label', unit: 'percent', decimals: 5, available: true, reason: null };
const registry = [factories, army, power, stability];
const identity = (firstHour: number, lastHour: number, slot: number, countryId = slot + 1) => ({ firstHour, lastHour, slot, countryId });
function series(tag: string, slot: number, first: string | null, second = first, firstHour = 10, lastHour = 12): ScoreboardSeries {
  return { tag, segments: [{ firstHour, lastHour }], identities: [identity(firstHour, lastHour, slot)], membership: null,
    columns: [{ key: 'factories', runs: [{ firstHour, lastHour: firstHour, amount: first, startsSegment: true },
      ...(lastHour > firstHour ? [{ firstHour: firstHour + 1, lastHour, amount: second, startsSegment: false }] : [])],
    readRuns: [{ firstHour, lastHour, readHour: firstHour, hourly: true }] }] };
}
const DEFAULT_COLUMNS = ['factories', 'armySize'] as const;
const TEST_SORT = { column: 'factories', direction: 'descending' } as const;
const reply = (rows: ScoreboardSeries[]): ScoreboardReply => ({ branchId: 'branch', defaultColumns: [...DEFAULT_COLUMNS], defaultSort: 'factories', columns: registry, series: rows, warnings: [] });

test('Scoreboard follows playback and uses the registry defaults with top ten', () => {
  assert.equal(definition.history, false);
  assert.equal(definition.defaultTime, 'follow');
  assert.equal(definition.defaultCountries.kind, 'group');
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const columnsOption = OPTIONS.find(option => option.key === 'columns');
  assert.ok(columnsOption?.kind === 'content-set');
  assert.equal('max' in columnsOption, false);
  assert.deepEqual(DEFAULT_SORT, { column: '', direction: 'descending' });
  assert.deepEqual(selectedColumns(registry, '', DEFAULT_COLUMNS).map(column => column.key), ['factories', 'armySize']);
  assert.deepEqual(selectedColumns(registry, 'stability factories', DEFAULT_COLUMNS).map(column => column.key), ['stability', 'factories']);
  assert.deepEqual(scoreboardSort(registry, [army], TEST_SORT, 'factories'), { column: 'country', direction: 'descending' });
});

test('registry owns labels and only requested measures plus the ranking measure are asked for', () => {
  const resolved = { tags: [], membership: null, unobserved: [], dropped: 0, reason: null, top: 10 };
  assert.deepEqual(scoreboardRequest('branch', resolved, [power], 'factories', 'descending'), {
    branchId: 'branch', columns: ['politicalPower', 'factories'], tags: [], top: { count: 10, sort: 'factories', descending: true },
  });
  assert.deepEqual(rankingSort(registry, { column: 'country', direction: 'ascending' }, 'armySize', 'factories'), TEST_SORT);
  assert.deepEqual(rankingSort(registry, { column: 'country', direction: 'ascending' }, 'politicalPower', 'factories'), { column: 'politicalPower', direction: 'descending' });
  assert.deepEqual(tableSortOf(tableSortToken({ column: 'politicalPower', direction: 'ascending' }), DEFAULT_SORT), { column: 'politicalPower', direction: 'ascending' });
});

test('unavailable sorting keeps an available top measure descending instead of alphabetical membership', () => {
  const model = scoreboardModel(reply([series('a', 0, '1'), series('z', 1, '9')]));
  for (const direction of ['ascending', 'descending'] as const) {
    const ranking = rankingSort(registry, { column: 'armySize', direction }, 'armySize', 'factories');
    assert.deepEqual(ranking, TEST_SORT);
    assert.equal(scoreboardRows(model, model.tags, 10, ranking.column, ranking.direction, 1)[0].tag, 'z');
    assert.deepEqual(rankingSort(registry, { column: 'armySize', direction }, 'politicalPower', 'factories'),
      { column: 'politicalPower', direction: 'descending' });
  }
  assert.deepEqual(rankingSort(registry, { column: 'politicalPower', direction: 'ascending' }, '', 'factories'),
    { column: 'politicalPower', direction: 'ascending' });
});

test('exact bigint values re-rank hourly with current-tag ties and missing last in either direction', () => {
  const model = scoreboardModel(validateScoreboardReply(reply([
    series('é', 0, '9007199254740993', '7'), series('z', 1, '9007199254740992', '8'), series('missing', 2, null), series('zero', 3, '0'),
  ])));
  const before = scoreboardRows(model, model.tags, 10, 'factories', 'descending');
  assert.deepEqual(before.map(row => row.tag), ['é', 'z', 'zero', 'missing']);
  const after = scoreboardRows(model, model.tags, 11, 'factories', 'descending');
  assert.deepEqual(after.map(row => row.tag), ['z', 'é', 'zero', 'missing']);
  assert.deepEqual(scoreboardRows(model, model.tags, 10, 'factories', 'ascending').map(row => row.tag), ['zero', 'z', 'é', 'missing']);
  const previous = rankChanges(new Map(), before.map(row => row.key), 0);
  const changed = rankChanges(previous, after.map(row => row.key), 100);
  assert.equal(changed.get(after[0].key)?.delta, 1);
  assert.equal(changed.get(after[1].key)?.delta, -1);
  assert.equal(changed.get(after[0].key)?.until, 2600);
  assert.equal(rankChanges(changed, after.map(row => row.key), 2700).get(after[0].key)?.delta, 0);
  assert.equal(rankChanges(changed, after.map(row => row.key), 200, 2500, true).get(after[0].key)?.delta, 0);
});

test('renamed native identities keep their row key, recycled tags and disappearing countries do not', () => {
  const old = series('old opaque', 5, '4', '4', 10, 10);
  const renamed = series('new opaque', 5, '4', '4', 11, 12);
  const recycled = series('old opaque', 8, '9', '9', 11, 12);
  const model = scoreboardModel(reply([old, renamed]));
  const before = scoreboardRows(model, model.tags, 10, 'factories', 'descending');
  const after = scoreboardRows(model, model.tags, 11, 'factories', 'descending');
  assert.equal(before[0].key, after[0].key);
  assert.equal(after[0].tag, 'new opaque');
  assert.equal(after[0].hour, 11);
  assert.deepEqual(scoreboardRows(model, model.tags, 13, 'factories', 'descending'), []);
  assert.notEqual(before[0].key, scoreboardRows(scoreboardModel(reply([recycled])), ['old opaque'], 11, 'factories', 'descending')[0].key);
});

test('read gaps never carry a value and held reads stay dated in the tooltip', () => {
  const entry = series('opaque', 1, '12', '12', 10, 35);
  entry.columns[0].readRuns = [{ firstHour: 10, lastHour: 33, readHour: 10, hourly: false }, { firstHour: 35, lastHour: 35, readHour: 35, hourly: true }];
  const model = scoreboardModel(validateScoreboardReply(reply([entry])));
  const row = scoreboardRows(model, model.tags, 33, 'factories', 'descending')[0];
  assert.equal(row.cells.get('factories')?.readHour, 10);
  assert.equal(row.cells.get('factories')?.amount, 12n);
  const gap = scoreboardRows(model, model.tags, 34, 'factories', 'descending')[0];
  assert.equal(gap.cells.get('factories')?.amount, null);
  assert.match(scoreboardTooltip(gap, [factories], 'Name').rows[1].label, /Not recorded/);
  const unknownModel = scoreboardModel(reply([series('unknown', 2, null)]));
  const unknown = scoreboardRows(unknownModel, unknownModel.tags, 10, 'factories', 'descending')[0];
  assert.match(scoreboardTooltip(unknown, [factories], 'Name').rows[1].label, /Not known/);
  const tooltip = scoreboardTooltip(row, [factories, army], 'Name at row hour');
  assert.equal(tooltip.title, 'Name at row hour');
  assert.match(tooltip.rows[1].label, /^Read /);
  assert.match(tooltip.rows.at(-1)!.label, /Not available yet/);
});

test('formatting retains every recorded decimal and exact source units', () => {
  assert.equal(formatCell(factories, 9007199254740993n), '9,007,199,254,740,993');
  assert.equal(formatCell(power, 12345678n), '123.45678');
  assert.equal(formatCell(stability, 12345n), '12.345%');
  assert.equal(formatCell(power, -1n), '−0.00001');
  assert.equal(formatCell(power, null), '—');
  assert.equal(formatCell(power, 0n), '0');
});

test('invalid payloads are refused without repairing values or dropping countries', () => {
  const valid = reply([series('opaque', 1, '5')]);
  assert.equal(validateScoreboardReply(valid), valid);
  const invalid = structuredClone(valid);
  invalid.series[0].columns[0].runs[0].amount = '5.1';
  assert.throws(() => validateScoreboardReply(invalid), /values/);
  assert.throws(() => validateScoreboardReply({ ...valid, columns: [...registry, factories] }), /column/);
  assert.throws(() => validateScoreboardReply({ ...valid, series: [...valid.series, valid.series[0]] }), /country/);
  const badRead = structuredClone(valid);
  badRead.series[0].columns[0].readRuns[0].readHour = 11;
  assert.throws(() => validateScoreboardReply(badRead), /values/);
});

test('five thousand countries sort, re-rank, and retain missing countries without player gates', () => {
  const input = Array.from({ length: 5000 }, (_, i) => series(`opaque-${i.toString().padStart(4, '0')}`, i, i % 100 === 0 ? null : String(i), String(5000 - i)));
  const model = scoreboardModel(validateScoreboardReply(reply(input)));
  const at = scoreboardRows(model, model.tags, 10, 'factories', 'descending', 10);
  assert.equal(at.length, 10);
  assert.equal(at[0].tag, 'opaque-4999');
  assert.equal(scoreboardRows(model, model.tags, 11, 'factories', 'descending', 10)[0].tag, 'opaque-0000');
  const all = scoreboardRows(model, model.tags, 10, 'factories', 'ascending');
  assert.equal(all.length, 5000);
  assert.equal(all[0].tag, 'opaque-0001');
  assert.equal(all.at(-1)?.tag, 'opaque-4900');
  const missing = sortRows(all, () => null, 'descending', row => row.key, row => row.tag);
  assert.equal(missing[0].tag, 'opaque-0000');
  const request = scoreboardRequest('branch', { tags: model.tags, membership: null, unobserved: [], dropped: 0, reason: null }, [factories], 'factories', 'descending');
  assert.deepEqual(request.tags, model.tags);
  assert.equal(request.top, null);
  const topRequest = scoreboardRequest('branch', { tags: [], membership: null, unobserved: [], dropped: 0, reason: null, top: 5000 }, [factories], 'factories', 'descending');
  assert.equal(topRequest.top?.count, 5000);
});
