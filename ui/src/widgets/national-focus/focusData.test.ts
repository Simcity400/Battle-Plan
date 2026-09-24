import assert from 'node:assert/strict';
import test from 'node:test';
import { focusAt, focusCompletion, focusProgressAt, focusRequest, validateFocusReply } from './focusData.ts';
import { loadProduct } from '../../dashboard/products.ts';
import type { DashboardClient, ProductRequest } from '../../dashboard/client';
import type { FocusReply } from '../../generated/dashboard';

const request = focusRequest('source', 'v1', { branchId: '1', tag: 'GER' });
const first = { countryName: 'Germany', completed: [{ ordinal: 0, token: 'one', completedByHour: null }], current: 'two', continuous: null, paused: false };
const reply: FocusReply = { history: { segments: [
  { firstHour: 100, lastHour: 123, order: first },
  { firstHour: 125, lastHour: 200, order: { ...first, completed: [...first.completed, { ordinal: 1, token: 'two', completedByHour: 125 }], current: null } },
], progress: [
  { firstHour: 100, lastHour: 110, progressRaw: '6800000' },
  { firstHour: 111, lastHour: 123, progressRaw: '6900000' },
  { firstHour: 125, lastHour: 140, progressRaw: '100000' },
  { firstHour: 141, lastHour: 160, progressRaw: '200000' },
] }, artwork: [], warnings: [] };

test('focus histories reject overlap, future completion bounds and invalid artwork', () => {
  assert.equal(validateFocusReply(reply), reply);
  const invalid = (firstHour: number, lastHour: number, ordinal: number, completedByHour: number | null) => ({ ...reply, history: { progress: [], segments: [{ firstHour, lastHour, order: { ...first, completed: [{ ordinal, token: 'one', completedByHour }] } }] } });
  for (const bad of [invalid(100, 99, 0, null), invalid(100, 100, 3, null), invalid(100, 100, 0, 101), invalid(NaN, 100, 0, null)]) assert.throws(() => validateFocusReply(bad));
  assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, segments: [...reply.history.segments, reply.history.segments[0]] } }));
  for (const progressRaw of ['1.5', '', '99999999999999999999']) assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, progress: [{ firstHour: 1, lastHour: 2, progressRaw }] } }));
  assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, progress: [...reply.history.progress, reply.history.progress[0]] } }));
  const art = { token: 'one', name: 'One', description: 'Game description', baseDurationDays: 70, icon: null };
  assert.equal(validateFocusReply({ ...reply, artwork: [art] }).artwork[0].baseDurationDays, 70);
  for (const baseDurationDays of [-1, NaN, Infinity]) assert.throws(() => validateFocusReply({ ...reply, artwork: [{ ...art, baseDurationDays }] }));
  for (const icon of ['https://example.com/remote.png', 'data:text/html;base64,YQ==']) assert.throws(() => validateFocusReply({ ...reply, artwork: [{ ...art, icon }] }));
});

test('exact local playback preserves unknown dates and never bridges gaps or leaks future focuses', () => {
  assert.equal(focusAt(reply.history, 99), null);
  assert.equal(focusAt(reply.history, 100), first);
  assert.equal(focusAt(reply.history, 123)?.completed.length, 1);
  assert.equal(focusAt(reply.history, 124), null);
  assert.equal(focusAt(reply.history, 125)?.completed[1].completedByHour, 125);
  assert.equal(focusAt(reply.history, 201), null);
});

test('many focus widgets share one load while playback advances without requests', async () => {
  let calls = 0;
  const client = { product: async (input: ProductRequest) => { calls++; return { sourceId: input.sourceId, revision: input.revision, product: input.product, data: reply }; } } as unknown as DashboardClient;
  const results = await Promise.all(Array.from({ length: 24 }, () => loadProduct<FocusReply>(client, request, new AbortController().signal, validateFocusReply)));
  assert.equal(calls, 1);
  assert.ok(results.every(value => value === results[0]));
  for (let hour = 100; hour <= 200; hour++) for (const result of results) focusAt(result.history, hour);
  assert.equal(calls, 1);
  await loadProduct(client, focusRequest('source', 'v1', { branchId: '1', tag: 'ENG' }), new AbortController().signal, validateFocusReply);
  assert.equal(calls, 2);
});

test('focus progress reports exact days and days left only from a known daily rate', () => {
  // The second day of `two`: 69 of 70 days done, one day gained, one day left.
  const late = focusProgressAt(reply.history, 115)!;
  assert.deepEqual(late, { progress: 6_900_000n, dailyGain: 100_000n });
  assert.deepEqual(focusCompletion(late, 70), { fraction: 0.9857, elapsedDays: 69, totalDays: 70, daysLeft: 1 });
  // The first recorded day has no earlier day to measure a rate from.
  assert.equal(focusProgressAt(reply.history, 105)!.dailyGain, null);
  assert.equal(focusCompletion(focusProgressAt(reply.history, 105)!, 70)!.daysLeft, null);
  // Hour 124 is a gap: nothing is reported there, and the next run never measures across it.
  assert.equal(focusProgressAt(reply.history, 124), null);
  assert.equal(focusProgressAt(reply.history, 130)!.dailyGain, null);
  // No current focus after 125, so banked progress has no rate toward a focus.
  assert.equal(focusProgressAt(reply.history, 150)!.dailyGain, null);
  // A faster rate shortens the estimate; completion clamps to the base duration.
  assert.equal(focusCompletion({ progress: 3_500_000n, dailyGain: 125_000n }, 70)!.daysLeft, 28);
  assert.equal(focusCompletion({ progress: 9_000_000n, dailyGain: 100_000n }, 70)!.fraction, 1);
  assert.equal(focusCompletion({ progress: 0n, dailyGain: null }, 0), null);
});
