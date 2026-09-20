import assert from 'node:assert/strict';
import test from 'node:test';
import { focusAt, focusReplyBytes, validateFocusReply } from './focusData.ts';
import { countryData } from './countryData.ts';
import type { DashboardClient, FocusReply, FocusRequest } from './client';

const query: FocusRequest = { sourceId: 'source', revision: 'v1', branchId: '1', tag: 'GER' };
const first = { countryName: 'Germany', completed: [{ ordinal: 0, token: 'one', completedByHour: null }], current: 'two', continuous: null, paused: false };
const reply: FocusReply = { ...query, history: { segments: [
  { firstHour: 100, lastHour: 123, order: first },
  { firstHour: 125, lastHour: 200, order: { ...first, completed: [...first.completed, { ordinal: 1, token: 'two', completedByHour: 125 }], current: null } },
] }, artwork: [], warnings: [] };

test('focus histories reject stale identity, overlap, future completion bounds and invalid artwork', () => {
  assert.equal(validateFocusReply(reply, query), reply);
  for (const changes of [{ tag: 'ENG' }, { branchId: '2' }, { revision: 'v2' }, { sourceId: 'other' }]) assert.throws(() => validateFocusReply({ ...reply, ...changes }, query));
  const invalid = (firstHour: number, lastHour: number, ordinal: number, completedByHour: number | null) => ({ ...reply, history: { segments: [{ firstHour, lastHour, order: { ...first, completed: [{ ordinal, token: 'one', completedByHour }] } }] } });
  for (const bad of [invalid(100, 99, 0, null), invalid(100, 100, 3, null), invalid(100, 100, 0, 101), invalid(NaN, 100, 0, null)]) assert.throws(() => validateFocusReply(bad, query));
  assert.throws(() => validateFocusReply({ ...reply, history: { segments: [...reply.history.segments, reply.history.segments[0]] } }, query));
  const art = { token: 'one', name: 'One', description: 'Game description', baseDurationDays: 70, icon: null };
  assert.equal(validateFocusReply({ ...reply, artwork: [art] }, query).artwork[0].baseDurationDays, 70);
  for (const baseDurationDays of [-1, NaN, Infinity]) assert.throws(() => validateFocusReply({ ...reply, artwork: [{ ...art, baseDurationDays }] }, query));
  for (const icon of ['https://example.com/remote.png', 'data:text/html;base64,YQ==']) assert.throws(() => validateFocusReply({ ...reply, artwork: [{ ...art, icon }] }, query));
});

test('exact local playback preserves unknown dates and never bridges gaps or leaks future focuses', () => {
  assert.equal(focusAt(reply.history, 99), null);
  assert.equal(focusAt(reply.history, 100), first);
  assert.equal(focusAt(reply.history, 123)?.completed.length, 1);
  assert.equal(focusAt(reply.history, 124), null);
  assert.equal(focusAt(reply.history, 125)?.completed[1].completedByHour, 125);
  assert.equal(focusAt(reply.history, 201), null);
});

test('many focus widgets share preparation while playback advances without requests', async () => {
  let calls = 0;
  const client = { focusHistory: async (request: FocusRequest) => { calls++; return { ...reply, ...request }; } } as unknown as DashboardClient;
  const cache = countryData(client);
  const results = await Promise.all(Array.from({ length: 24 }, () => cache.focus(query, new AbortController().signal)));
  assert.equal(calls, 1);
  assert.ok(results.every(value => value === results[0]));
  for (let hour = 100; hour <= 200; hour++) for (const result of results) focusAt(result.history, hour);
  assert.equal(calls, 1);
  await cache.focus({ ...query, tag: 'ENG' }, new AbortController().signal);
  await cache.focus(query, new AbortController().signal);
  assert.equal(calls, 2);
  cache.release(query.sourceId);
  await cache.focus(query, new AbortController().signal);
  assert.equal(calls, 3);
});

test('focus cache accounting includes long localisation, warnings and decoded artwork', () => {
  const longer = { ...reply, artwork: [{ token: 'one', name: 'Title', description: 'x'.repeat(8192), baseDurationDays: 70, icon: 'data:image/png;base64,YQ==' }], warnings: ['w'.repeat(8192)] };
  assert.ok(focusReplyBytes(longer) - focusReplyBytes(reply) >= 4 * 8192 + 96 * 96 * 4);
});
