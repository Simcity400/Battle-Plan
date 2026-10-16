import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { ScoreboardColumn, ScoreboardPreparedParams, ScoreboardPreparedReply, ScoreboardSeries } from '../../generated/dashboard';
import { loadProduct } from '../../dashboard/products.ts';
import { barFraction } from '../../charts/table/logic.ts';
import { preparedScoreboardRows, queryScoreboard, scoreboardCellPage, scoreboardRegistryRequest, validatePreparedScoreboard } from './preparedScoreboard.ts';

const factories: ScoreboardColumn = { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null };
const losses: ScoreboardColumn = { key: 'casualtiesTaken', label: 'Casualties taken', unit: 'people', decimals: 0, available: true, reason: null };
const request: ScoreboardPreparedParams = { selection: { branchId: '1', columns: [factories.key, losses.key], tags: [], top: { count: 5000, sort: factories.key, descending: true } },
  hour: 10, sort: factories.key, descending: true, index: true, bars: true };
function entry(tag: string, slot: number, key = factories.key, amount: string | null = '1', hour = 10): ScoreboardSeries {
  return { tag, segments: [{ firstHour: hour, lastHour: hour }], identities: [{ firstHour: hour, lastHour: hour, slot, countryId: slot + 10 }], membership: null,
    columns: [{ key, runs: [{ firstHour: hour, lastHour: hour, amount, startsSegment: false }], readRuns: [{ firstHour: hour, lastHour: hour, readHour: hour, hourly: true }] }] };
}
function reply(rows: ScoreboardSeries[], hour = 10): ScoreboardPreparedReply {
  return { hour, maxima: { factories: '5000', casualtiesTaken: '9007199254740993' },
    data: { branchId: '1', defaultColumns: [factories.key], defaultSort: factories.key, columns: [factories, losses], series: rows, warnings: [] } };
}

test('prepared moment index keeps 5000 identities while visible cells and bars remain exact', () => {
  const index = validatePreparedScoreboard(reply(Array.from({ length: 5000 }, (_, slot) => entry(`opaque-${slot}`, slot, factories.key, String(slot + 1)))));
  for (const row of index.data.series) row.columns[0].readRuns = [];
  const row = entry('opaque-4999', 4999, losses.key, '-9007199254740993');
  row.columns[0].runs[0].atLeast = true;
  row.columns[0].readRuns[0] = { firstHour: 10, lastHour: 10, readHour: 7, hourly: false };
  const page = validatePreparedScoreboard(reply([row]));
  const rows = preparedScoreboardRows(index, page, factories.key, true);
  assert.equal(rows.length, 5000);
  assert.equal(rows[0].tag, 'opaque-4999');
  assert.deepEqual(rows[0].cells.get(losses.key), { amount: -9007199254740993n, readHour: 7, atLeast: true });
  assert.equal(rows[1].cells.has(losses.key), false);
  assert.equal(rows[1].cells.get(factories.key)?.readHour, null, 'index quantities never invent a supplying date');
  assert.equal(barFraction(rows[0].cells.get(factories.key)?.amount, BigInt(index.maxima.factories!)), 1);
  assert.deepEqual(scoreboardCellPage(request, ['opaque-4999', 'opaque-4999']), {
    ...request, index: false, bars: false, selection: { ...request.selection, top: null, tags: ['opaque-4999'] },
  });
});

test('pages cannot move values between supplying hours, branches, tags or reused identities', () => {
  const index = reply([entry('a', 1)]);
  const wrongIdentity = reply([entry('a', 2, losses.key, '123')]);
  const renamed = reply([entry('b', 1, losses.key, '123')]);
  for (const page of [wrongIdentity, renamed, reply([entry('a', 1, losses.key, '123', 11)], 11), { ...reply([entry('a', 1, losses.key, '123')]), data: { ...index.data, branchId: '2' } }]) {
    assert.equal(preparedScoreboardRows(index, page, factories.key, true)[0].cells.has(losses.key), false);
  }
  const invalid = reply([entry('a', 1)]);
  invalid.data.series[0].identities[0].lastHour = 11;
  invalid.data.series[0].segments[0].lastHour = 11;
  assert.throws(() => validatePreparedScoreboard(invalid), /moment/);
  assert.throws(() => validatePreparedScoreboard({ ...index, maxima: { factories: '-1' } }), /moment/);
});

test('scoreboard registry and moment queries use prepared coverage and never invoke the oracle', async () => {
  const source = { id: 's', revision: 'r' } as DashboardSource;
  const seen: unknown[] = [];
  const value = reply([entry('a', 1)]);
  const client = { product: () => { throw new Error('old scoreboard scan'); },
    preparedQuery: async (_source: DashboardSource, product: string, params: unknown) => {
      assert.equal(product, 'scoreboard'); seen.push(params); return value;
    } } as unknown as DashboardClient;
  const signal = new AbortController().signal;
  const registry = scoreboardRegistryRequest('1');
  await loadProduct(client, { sourceId: source.id, revision: source.revision, product: 'scoreboard', params: registry }, signal, undefined, source);
  assert.deepEqual(seen[0], registry);
  assert.deepEqual(await queryScoreboard(client, source, request, signal), value);
  const controller = new AbortController(); controller.abort();
  await assert.rejects(queryScoreboard(client, source, request, controller.signal), /cancelled/);
  assert.equal(seen.length, 2, 'cancelled queries must not issue another request');
});

test('a 5000-country cell request rejects a partial successful reply in one request', async () => {
  const source = { id: 'incomplete', revision: '1' } as DashboardSource;
  const tags = Array.from({ length: 5000 }, (_, slot) => `country-${slot}`);
  const cells = scoreboardCellPage(request, tags);
  let requests = 0;
  const client = { preparedQuery: async () => {
    assert.equal(++requests, 1, 'a no-progress reply must not restart the request');
    return reply([entry(tags[0], 0)]);
  } } as unknown as DashboardClient;
  await assert.rejects(queryScoreboard(client, source, cells, new AbortController().signal), /did not supply/);
  assert.equal(requests, 1);
});
