import test from 'node:test';
import assert from 'node:assert/strict';
import { approximateAt, gateMoved, pendingReplyAt, preparedPending, pendingAt, preparationCoverage, preparedPrefix, preparedThrough, unobservedOrPending } from './preparedPrefix.ts';
import { validatePeaceDealsReply } from './peaceDealsData.ts';
import { deal, reply } from '../widgets/territory-transfers/transfers.fixture.ts';

const coverage = { schema: 24, firstHour: 0, lastHour: 99, complete: false };

test('a revisable prefix stays approximate even for a pinned hour with no pending tail', () => {
  const prefix = preparedPrefix({ coverage, pending: [], approximate: [{ firstHour: 0, lastHour: 99 }] });
  assert.equal(pendingAt(prefix, 20), false);
  assert.equal(approximateAt(prefix, 20), true);
  assert.equal(approximateAt(prefix, 100), false);
  assert.throws(() => preparedPrefix({ coverage, pending: [], approximate: [{ firstHour: 5, lastHour: 4 }] }), /malformed/);
});

test('a cold prefix ends before the first hour without claiming an accepted hour', () => {
  const empty = { coverage: { firstHour: 100, lastHour: 99, complete: false }, pending: [{ firstHour: 100, lastHour: 200 }] };
  assert.deepEqual(preparedPrefix(empty), empty);
  assert.equal(pendingAt(preparedPrefix(empty), 100), true);
  assert.throws(() => preparedPrefix({ ...empty, coverage: { ...empty.coverage, lastHour: 98 } }), /malformed/);
  assert.throws(() => preparedPrefix({ ...empty, coverage: { ...empty.coverage, complete: true } }), /malformed/);
});

test('a reply answered from a prepared prefix carries its coverage and the hours still pending', () => {
  assert.equal(preparedPrefix(null), null);
  assert.equal(preparedPrefix({ rows: [] }), null);
  // Coverage alone (an event list without the field, or a sampled series) is not a prefix reply.
  assert.equal(preparedPrefix({ coverage, rows: [] }), null);
  const prefix = preparedPrefix({ coverage, pending: [{ firstHour: 100, lastHour: 149 }, { firstHour: 160, lastHour: 199 }], rows: [] });
  assert.deepEqual(prefix, { coverage, pending: [{ firstHour: 100, lastHour: 149 }, { firstHour: 160, lastHour: 199 }] });
  assert.equal(pendingAt(prefix, 99), false);
  assert.equal(pendingAt(prefix, 100), true);
  assert.equal(pendingAt(prefix, 155), false, 'a recorded gap is not pending');
  assert.equal(pendingAt(prefix, 199), true);
  assert.equal(pendingAt(null, 150), false);
  for (const malformed of [{ coverage: null, pending: [] }, { coverage, pending: null },
    { coverage, pending: [{ firstHour: 160, lastHour: 199 }, { firstHour: 100, lastHour: 149 }] }, { coverage, pending: [{ firstHour: 2, lastHour: 1 }] }])
    assert.throws(() => preparedPrefix(malformed), /malformed/);
});

test('hours still being prepared join recorded gaps where a span beside them is not a change', () => {
  const gaps = [{ firstHour: 10, lastHour: 19 }, { firstHour: 150, lastHour: 159 }];
  const prefix = { coverage, pending: [{ firstHour: 100, lastHour: 149 }, { firstHour: 160, lastHour: 199 }] };
  assert.deepEqual(unobservedOrPending(gaps, prefix), [{ firstHour: 10, lastHour: 19 }, { firstHour: 100, lastHour: 149 },
    { firstHour: 150, lastHour: 159 }, { firstHour: 160, lastHour: 199 }]);
  assert.deepEqual(unobservedOrPending(gaps, null), gaps);
  assert.notEqual(unobservedOrPending(gaps, null), gaps);
});

test('totals read through the earliest prefix that leaves the hour shown pending', () => {
  const partial = (lastHour: number, pending: boolean) => ({ coverage: { ...coverage, lastHour }, pending: pending ? [{ firstHour: lastHour + 1, lastHour: 400 }] : [] });
  assert.equal(preparedThrough(undefined, 300), 300);
  assert.equal(preparedThrough([{ rows: [] }], 300), 300);
  assert.equal(preparedThrough([partial(200, true), partial(150, true)], 300), 150);
  assert.equal(preparedThrough([partial(350, false)], 300), 300, 'a prefix reaching the hour leaves nothing pending');
});

test('a peace deal answered from a prefix names the outcome still being prepared; older replies name none', () => {
  const settled = reply([deal('conference', [])]);
  delete (settled.deals[0] as { preparing?: string[] }).preparing;
  assert.deepEqual(validatePeaceDealsReply(settled).deals[0].preparing, []);
  const open = reply([deal('conference', [], { preparing: ['outcome'] })]);
  assert.deepEqual(validatePeaceDealsReply(open).deals[0].preparing, ['outcome']);
  assert.throws(() => validatePeaceDealsReply(reply([deal('conference', [], { preparing: [7] as unknown as string[] })])), /malformed/);
});

test('a prefix reply asks again once the coverage it waits for moves past it', () => {
  const at = (lastHour: number, complete = false) => ({ firstHour: 0, lastHour, complete });
  const status = (state: 'preparing' | 'ready', walk: Record<string, ReturnType<typeof at>>, families?: Record<string, Record<string, ReturnType<typeof at>>>) =>
    ({ sourceId: 's', revision: 'r', requestId: 'q', backgroundEnabled: true, state, sections: [], coverage: walk, families, bytes: '0', error: null });
  const reply = at(99);
  assert.equal(gateMoved(status('preparing', { 1: at(99) }), undefined, '1', reply), false);
  assert.equal(gateMoved(status('preparing', { 1: at(120) }), ['walk'], '1', reply), true);
  // A family's own coverage, not the walk's, moves a reply it gates.
  assert.equal(gateMoved(status('preparing', { 1: at(500) }, { production: { 1: at(99) } }), ['production'], '1', reply), false);
  assert.equal(gateMoved(status('preparing', {}, { production: { 1: at(150) } }), ['production'], '1', reply), true);
  // A joint gate moves with its slowest entry.
  assert.equal(gateMoved(status('preparing', {}, { a: { 1: at(150) }, b: { 1: at(99) } }), ['a', 'b'], '1', reply), false);
  assert.equal(gateMoved(status('preparing', {}, { a: { 1: at(150, true) }, b: { 1: at(99, true) } }), ['a', 'b'], '1', reply), true);
  // A status that lists none of it moves once preparation is ready.
  assert.equal(gateMoved(status('preparing', {}), ['production'], '1', reply), false);
  assert.equal(gateMoved(status('ready', {}), ['production'], '1', reply), true);
});


test('preparation coverage leaves the cipher capture coverage untouched', () => {
  const captured = [{ firstHour: 100, lastHour: 123, complete: true, reason: null }];
  const reply = { coverage: captured, preparedCoverage: coverage, pending: [{ firstHour: 124, lastHour: 150 }] };
  assert.deepEqual(preparedPrefix(reply), { coverage, pending: reply.pending });
  assert.equal(preparationCoverage(reply), coverage);
  assert.equal(reply.coverage, captured);
  assert.equal(preparationCoverage({ coverage: captured }), undefined);
  assert.equal(preparationCoverage({ coverage: null }), undefined);
  assert.throws(() => preparedPrefix({ coverage: null, pending: [] }), /malformed/);
  assert.throws(() => preparedPrefix({ preparedCoverage: null, coverage, pending: [] }), /malformed/);
});


test('a cold first view has pending hours without inventing a prepared hour', () => {
  const empty = { firstHour: 100, lastHour: 99, complete: false };
  const value = { preparedCoverage: empty, coverage: [], pending: [{ firstHour: 100, lastHour: 123 }] };
  assert.deepEqual(preparedPrefix(value), { coverage: empty, pending: value.pending });
  assert.equal(preparationCoverage(value), empty);
  assert.equal(pendingAt(preparedPrefix(value), 100), true);
});


test('the shared cold parser preserves approximate spans and accepts empty named extents', () => {
  const approximate = [{ firstHour: 100, lastHour: 123 }];
  const pending = [{ firstHour: 124, lastHour: 150 }];
  const empty = { firstHour: 100, lastHour: 99, complete: false };
  assert.deepEqual(preparedPrefix({ preparedCoverage: empty, coverage: [], pending, approximate }), { coverage: empty, pending, approximate });
  assert.equal(preparationCoverage({ preparedCoverage: empty }), empty);
  assert.deepEqual(preparedPrefix({ coverage: empty, pending, approximate }), { coverage: empty, pending, approximate });
  assert.equal(approximateAt(preparedPrefix({ coverage: empty, pending, approximate }), 100), true);
  assert.throws(() => preparedPrefix({ preparedCoverage: null, pending, approximate: [{ firstHour: 3, lastHour: 2 }] }), /malformed/);
});

test('cold replies have pending hours without pretending any coverage was accepted', () => {
  const cold = { coverage: { firstHour: 100, lastHour: 99, complete: false }, pending: [{ firstHour: 100, lastHour: 199 }] };
  assert.deepEqual(preparedPending(cold), cold.pending);
  assert.equal(pendingReplyAt(cold, 150), true);
  assert.equal(pendingReplyAt(cold, 99), false);
  assert.throws(() => preparedPending({ pending: [{ firstHour: 200, lastHour: 199 }] }), /malformed/);
});
