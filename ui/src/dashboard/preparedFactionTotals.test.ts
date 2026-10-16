import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeFactionReads, validatePreparedFactionTotals } from './preparedFactionTotals.ts';
import type { PreparedFactionTotals } from './preparedFactionTotals';
import type { DashboardClient, DashboardSource } from './client';
import { loadProduct } from './products.ts';

function reply(first: number, last: number, value: number): PreparedFactionTotals {
  const span = { firstHour: first, lastHour: last };
  return { branchId: '1', membershipMode: 'hour', unit: 'states', unavailableReason: null, warnings: [], recorded: [span], disjoint: [span],
    series: [{ id: '1', runs: [{ ...span, value, startsSegment: true }], names: [{ ...span, name: 'Faction', reason: null }],
      members: [{ ...span, tag: 'opaque alias' }], countedMembers: [{ ...span, tag: 'opaque alias' }], membership: [span], dissolutionHour: null, unavailableReason: null }] };
}

test('prepared faction reads retain exact head, hover and end values without overlapping runs', () => {
  const summary = reply(0, 10, -7);
  const head = reply(6, 6, 2), hover = reply(4, 4, 0), origin = reply(1, 1, -9);
  const result = validatePreparedFactionTotals(mergeFactionReads(head, hover, summary, origin));
  assert.deepEqual(result.series[0].runs.map(run => [run.firstHour, run.lastHour, run.value]),
    [[0, 0, -7], [1, 1, -9], [2, 3, -7], [4, 4, 0], [5, 5, -7], [6, 6, 2], [7, 10, -7]]);
  assert.equal(result.series[0].runs.at(-1)!.startsSegment, false, 'a clipped trailing piece does not invent a native break');
  assert.equal(result.series[0].countedMembers.length, 7);
  assert.equal(mergeFactionReads(undefined, hover, summary), undefined, 'geometry cannot establish the initial exact state');
});

test('dense faction envelopes never replace exact values or wide signed member amounts', () => {
  const summary = reply(10, 10, 5);
  summary.summaries = { '1': { from: 0, to: 10, width: 1, buckets: [{ firstHour: 0, lastHour: 10, firstValue: -500,
    lastValue: 600, lastValueHour: 10, minValue: -1000, minHour: 1, maxValue: 1000, maxHour: 2, firstBreak: true, hasBreak: false }] } };
  const head = reply(5, 5, -3);
  head.memberTracks = [{ tag: 'opaque alias', runs: [{ firstHour: 5, lastHour: 5, value: '-9007199254740993', startsSegment: false }],
    identities: [{ firstHour: 0, lastHour: 10, slot: 72, countryId: 88 }] }];
  const result = validatePreparedFactionTotals(mergeFactionReads(head, undefined, summary));
  assert.deepEqual(result.series[0].runs.map(run => run.value), [-3, 5]);
  assert.equal(result.memberTracks![0].runs[0].value, '-9007199254740993');
  assert.equal(result.memberTracks![0].identities[0].firstHour, 0, 'the full native census keeps stable colour slots');
  assert.equal(result.summaries!['1'].buckets[0]!.lastValue, 600);
  const broken = { ...summary, summaries: { '1': { ...summary.summaries['1'], width: 0 } } };
  assert.throws(() => validatePreparedFactionTotals(broken), /geometry/);
});

test('prepared faction member census keeps five thousand identities with unknown and zero values distinct', () => {
  const head = reply(5, 5, 0);
  head.memberTracks = Array.from({ length: 5000 }, (_, slot) => ({ tag: `country ${slot}`, identities: [{ firstHour: 0, lastHour: 10, slot, countryId: slot + 6000 }],
    runs: [{ firstHour: 5, lastHour: 5, value: slot % 2 ? null : '0', startsSegment: false }] }));
  const result = validatePreparedFactionTotals(mergeFactionReads(head));
  assert.equal(result.memberTracks!.length, 5000);
  assert.equal(result.memberTracks![0].runs[0].value, '0');
  assert.equal(result.memberTracks![1].runs[0].value, null);
});

test('the real faction loader waits through unchanged paced progress for 5000 members and bounds requests', { timeout: 3000 }, async () => {
  const source = { id: 'large-faction', revision: '1' } as DashboardSource;
  const data = reply(5, 5, 0);
  data.memberTracks = Array.from({ length: 5000 }, (_, slot) => ({ tag: `country ${slot}`, identities: [{ firstHour: 0, lastHour: 10, slot, countryId: slot + 6000 }],
    runs: [{ firstHour: 5, lastHour: 5, value: slot % 2 ? null : '0', startsSegment: false }] }));
  data.series[0].countedMembers = data.memberTracks.map(track => ({ tag: track.tag, firstHour: 5, lastHour: 5 }));
  const input = { sourceId: source.id, revision: source.revision, product: 'series-state', params: { branchId: '1',
    family: { kind: 'faction-totals', selection: { branchId: '1', source: { kind: 'territory', measure: 'states', rule: 'owned' }, membership: 'hour', top: 1, factionId: '1', parts: true } },
    reads: [{ hour: 5 }] } };
  let queries = 0, statuses = 0, ready = false;
  const client = {
    product: async () => { throw new Error('unexpected full-history faction request'); },
    preparedQuery: async (_source: DashboardSource, product: string, params: unknown) => {
      assert.ok(++queries <= 3, 'unchanged progress must not reread faction data');
      assert.equal(product, input.product); assert.deepEqual(params, input.params);
      return ready ? data : null;
    },
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 3, 'the finite preparation sequence needs at most three status requests');
      ready = statuses === 3;
      return { ...request, state: ready ? 'ready' : 'preparing', backgroundEnabled: true, bytes: '0', error: null,
        sections: [{ id: 'territory', label: 'Territory', state: ready ? 'ready' : 'preparing', done: ready ? '2' : '1', total: '2', error: null }] };
    },
  } as unknown as DashboardClient;
  const load = () => loadProduct(client, input, new AbortController().signal, validatePreparedFactionTotals, source);
  const results = await Promise.all(Array.from({ length: 32 }, () => load()));
  assert.ok(results.every(result => result === results[0]));
  assert.equal(results[0].memberTracks!.length, 5000);
  assert.equal(results[0].memberTracks![0].runs[0].value, '0');
  assert.equal(results[0].memberTracks![1].runs[0].value, null);
  assert.equal(await load(), results[0]);
  assert.equal(queries, 3); assert.equal(statuses, 3);
});
