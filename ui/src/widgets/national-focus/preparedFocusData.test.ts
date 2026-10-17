import assert from 'node:assert/strict';
import test from 'node:test';
import { lastPage, loadPreparedFocusPage, preparedArtKey, validatePreparedHour, validatePreparedOrder, validatePreparedArt, waitForFocusPreparation, canCarryPreparedFocus, loadPreparedFocusDetail, validateUnavailableFocus, loadPreparedFocusHour, focusTokenOrder, loadFocusArt } from './preparedFocusData.ts';
import type { FocusPreparedClient, PreparedFocusHour } from './preparedFocusData.ts';
import type { DashboardSource } from '../../dashboard/client';
const row: PreparedFocusHour = { schema: 6, slot: 7, countryId: 9001, hour: 100, lastHour: 123, orderFirstHour: 100, current: 'current', continuous: null, paused: false,
  completed: { first: '9007199254740993', count: '3', rows: '300' }, at: 101,
  progressRaw: '-9223372036854775808', dailyGainRaw: '18446744073709551615' };
const source = { id: 'source', revision: 'v1' } as DashboardSource;

test('focus first views accept a pending order date without inventing a completion date', () => {
  const sampled = { ...row, orderFirstHour: null, preparing: ['orderFirstHour', 'completedByHour'], completed: { sampled: true, first: '0', count: '1', rows: '1' } };
  assert.equal(validatePreparedHour(sampled, row.at), sampled);
  assert.throws(() => validatePreparedHour({ ...sampled, preparing: [] }, row.at), /Invalid prepared focus hour/);
  const entries = validatePreparedOrder([{ ordinal: 0, token: 'mod/focus', completedByHour: null }], sampled.completed, 0n);
  assert.equal(entries[0].completedByHour, null);
});
const art = (token: string) => ({ artwork: { token, name: token, description: null, baseDurationDays: 70, icon: null }, reason: null });
/** A client answering `focus-art` through `product`, each token's art from `named`. */
const artClient = (requests: { product: string; params: any }[], named: (token: string, params: any) => object = token => art(token),
  reply: (params: any) => object | null = () => null) => ({
  product: async (request: any) => {
    requests.push({ product: request.product, params: request.params });
    return { sourceId: request.sourceId, revision: request.revision, product: request.product,
      data: reply(request.params) ?? { art: request.params.tokens.map((token: string) => named(token, request.params)), warnings: ['Standing content notice'], failures: [] } };
  },
});
test('prepared focus validates exact integers, intervals and uncapped opaque values', () => {
  assert.equal(validatePreparedHour(row, 101), row);
  assert.equal(validatePreparedHour(null, 101), null);
  for (const value of [{ ...row, schema: 1 }, { ...row, schema: 5 }, { ...row, hour: 102 }, { ...row, lastHour: 100 }, { ...row, progressRaw: '1.5' },
    { ...row, at: 100 }, { ...row, completed: { ...row.completed, count: '2' } }]) assert.throws(() => validatePreparedHour(value, 101));
  const long = 'mod/'.repeat(1000);
  assert.equal(validatePreparedOrder([{ ordinal: 0, token: long, completedByHour: null }], { first: '0', count: '1', rows: '1' }, 0n)[0].token, long);
  assert.equal(validatePreparedArt([art(long)])[0].artwork.token, long);
  assert.throws(() => validatePreparedOrder([], row.completed, 0n));
  assert.throws(() => validatePreparedArt([{ artwork: null, reason: null }]));
  const hugeRows = 9007199254740993n * 128n;
  assert.equal(lastPage({ first: '0', count: '9007199254740993', rows: String(hugeRows) }), 9007199254740992n);
  // Tokens are asked in the service's order: by Unicode scalar value.
  assert.deepEqual(['b', '\u{1F600}', '～', 'a'].sort(focusTokenOrder), ['a', 'b', '～', '\u{1F600}']);
});
test('visible completion page and active tail read art at their own hours and independent static text', async () => {
  const requests: { product: string; params: any }[] = [];
  const client = { ...artClient(requests), preparedQuery: async (_source: DashboardSource, product: string, params: any) => {
    requests.push({ product, params });
    if (product === 'focus-order-page') return Array.from({ length: 128 }, (_, ordinal) => ({ ordinal, token: 'focus_' + ordinal, completedByHour: 100 }));
    if (product === 'focus-static-text') return params.tokens.map((token: string) => ({ ...art(token).artwork, description: 'Static ' + token }));
    throw new Error(product);
  } } as unknown as FocusPreparedClient;
  const result = await loadPreparedFocusPage(client, source, '1', 'opaque/mod/tag', 101, row, 0n, new AbortController().signal);
  assert.equal(result.order.length, 128);
  assert.equal(result.art.get(preparedArtKey(101, 'current'))?.artwork.name, 'current');
  assert.equal(requests.filter(item => item.product === 'focus-static-text').length, 2, '129 unique tokens use two static-text pages');
  assert.equal(result.art.get(preparedArtKey(100, 'focus_0'))?.artwork.description, 'Static focus_0');
  assert.equal(result.art.get(preparedArtKey(101, 'current'))?.artwork.description, 'Static current');
  assert.equal(result.staticText.length, 129);
  const arts = requests.filter(item => item.product === 'focus-art');
  assert.deepEqual(arts.map(item => [item.params.hour, item.params.tokens.length, item.params.mode]), [[100, 128, 'display'], [101, 1, 'display']]);
  assert.deepEqual(arts[0].params.tokens, [...arts[0].params.tokens].sort(focusTokenOrder));
  assert.deepEqual(result.warnings, ['Standing content notice']);
  assert.equal(requests[0].params.pages.first, '9007199254740993');
});
test('cancelled pages and omitted artwork never publish successful empty data', async () => {
  const controller = new AbortController();
  const client = { ...artClient([]), preparedQuery: async (_source: DashboardSource, product: string) => {
    controller.abort();
    return product === 'focus-order-page' ? [{ ordinal: 0, token: 'current', completedByHour: null }] : [art('current').artwork];
  } } as unknown as FocusPreparedClient;
  const one = { ...row, completed: { first: '0', count: '1', rows: '1' } };
  await assert.rejects(loadPreparedFocusPage(client, source, '1', 'tag', 101, one, 0n, controller.signal), /cancelled/);
  const omitting = artClient([], token => art(token), () => ({ art: [], warnings: [], failures: [] }));
  await assert.rejects(loadFocusArt(omitting as unknown as FocusPreparedClient, source, '1', 'tag', 101, ['a'], 'display', new AbortController().signal), /omitted/);
});

test('completion and active instances of one token use their own recorded hours', async () => {
  const requested = { ...row, hour: 120, lastHour: 120, at: 120, completed: { first: '0', count: '1', rows: '1' } };
  const dates: number[] = [];
  const client = { ...artClient([], (token, params) => { dates.push(params.hour); return { ...art(token), artwork: { ...art(token).artwork, name: 'At ' + params.hour } }; }),
    preparedQuery: async (_source: DashboardSource, product: string, params: any) => {
      if (product === 'focus-order-page') return [{ ordinal: 0, token: 'current', completedByHour: 100 }];
      if (product === 'focus-static-text') return params.tokens.map((token: string) => art(token).artwork);
      throw new Error(product);
    } } as unknown as FocusPreparedClient;
  const result = await loadPreparedFocusPage(client, source, '1', 'tag', 120, requested, 0n, new AbortController().signal);
  assert.equal(result.art.get(preparedArtKey(100, 'current'))?.artwork.name, 'At 100');
  assert.equal(result.art.get(preparedArtKey(120, 'current'))?.artwork.name, 'At 120');
  assert.deepEqual(dates.sort(), [100, 120]);
});

test('a refused or failed dated art request retains independent recorded order and names its reason', async () => {
  const one = { ...row, current: null, completed: { first: '0', count: '1', rows: '1' } };
  const query = { preparedQuery: async (_source: DashboardSource, product: string, params: any) => {
    if (product === 'focus-order-page') return [{ ordinal: 0, token: 'mod', completedByHour: 100 }];
    if (product === 'focus-static-text') return params.tokens.map((token: string) => art(token).artwork);
    throw new Error(product);
  } };
  const refused = { ...query, product: async () => { throw new Error('named content refusal'); } } as unknown as FocusPreparedClient;
  const result = await loadPreparedFocusPage(refused, source, '1', 'tag', 101, one, 0n, new AbortController().signal);
  assert.equal(result.order[0].token, 'mod');
  assert.equal(result.art.get(preparedArtKey(100, 'mod'))?.artwork.name, null);
  assert.equal(result.art.get(preparedArtKey(100, 'mod'))?.reason, 'named content refusal');
  const failed = { ...query, ...artClient([], token => art(token), () => ({ art: [], warnings: [], failures: [{ reason: 'Content stack refused', tokens: 1 }] })) } as unknown as FocusPreparedClient;
  const named = await loadPreparedFocusPage(failed, source, '1', 'tag', 101, one, 0n, new AbortController().signal);
  assert.equal(named.art.get(preparedArtKey(100, 'mod'))?.reason, 'Content stack refused');
});

test('explicit retry starts idle once and polls only queued/preparing states', async () => {
  const phases = ['idle', 'queued', 'preparing', 'ready'] as const;
  let status = 0, starts = 0, waits = 0;
  const client = { preparationStatus: async (request: any) => ({ ...request, state: phases[status++], sections: [], backgroundEnabled: false, bytes: '0', error: null }),
    prepareSource: async (request: any) => { starts++; return { ...request, state: 'queued', sections: [], backgroundEnabled: true, bytes: '0', error: null }; }
  } as unknown as FocusPreparedClient;
  await waitForFocusPreparation(client, source, new AbortController().signal, true, async () => { waits++; });
  assert.equal(status, 4); assert.equal(starts, 1); assert.equal(waits, 2);
});
test('failed display does not block ready numeric focus and terminal states stop polling', async () => {
  const client = { preparationStatus: async (request: any) => ({ ...request, state: 'failed', sections: [
    { id: 'focus', state: 'ready' }, { id: 'focus-display', state: 'failed' }], backgroundEnabled: true, bytes: '0', error: 'content unavailable' })
  } as unknown as FocusPreparedClient;
  await waitForFocusPreparation(client, source, new AbortController().signal);
  const failed = { preparationStatus: async (request: any) => ({ ...request, state: 'cancelled', sections: [], backgroundEnabled: true, bytes: '0', error: null }) } as unknown as FocusPreparedClient;
  await assert.rejects(waitForFocusPreparation(failed, source, new AbortController().signal), /cancelled/);
});

test('focus readiness stops an instant wait with repeated status', async () => {
  let statuses = 0, waits = 0;
  const client = { preparationStatus: async (request: object) => {
    assert.ok(++statuses <= 2, 'stationary focus preparation must not request a third status');
    return { ...request, state: 'preparing', sections: [{ id: 'focus', state: 'preparing', done: '1', total: '2' }],
      backgroundEnabled: true, bytes: String(statuses), error: null };
  } } as unknown as FocusPreparedClient;
  await assert.rejects(waitForFocusPreparation(client, source, new AbortController().signal, false, async () => { waits++; }), { message: 'Focus preparation made no progress.' });
  assert.deepEqual({ statuses, waits }, { statuses: 2, waits: 2 });
});

test('focus readiness keeps paced polling through unchanged status', async () => {
  let statuses = 0, waits = 0;
  const client = { preparationStatus: async (request: object) => {
    assert.ok(++statuses <= 3);
    return { ...request, state: statuses < 3 ? 'preparing' : 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null };
  } } as unknown as FocusPreparedClient;
  await waitForFocusPreparation(client, source, new AbortController().signal, false,
    async () => { waits++; await new Promise(resolve => setTimeout(resolve, 500)); });
  assert.deepEqual({ statuses, waits }, { statuses: 3, waits: 2 });
});

test('pending carry requires known native holder rather than reused tag', () => {
  assert.equal(canCarryPreparedFocus(row, 101), true);
  assert.equal(canCarryPreparedFocus(row, 124), false);
  assert.equal(canCarryPreparedFocus(row, { ...row, hour: 124, lastHour: 124 }), true);
  assert.equal(canCarryPreparedFocus(row, { ...row, countryId: 9002 }), false);
  assert.equal(canCarryPreparedFocus(row, { ...row, slot: 8 }), false);
});
test('prepared descriptions are text art at the confirming hour', async () => {
  const requests: { product: string; params: any }[] = [];
  const displayed = { ...row, hour: 120, lastHour: 120, at: 120 };
  const client = { ...artClient(requests, (token, params) => ({ ...art(token), artwork: { ...art(token).artwork, description: 'Description at ' + params.hour } })),
    preparedQuery: async (_source: DashboardSource, product: string) => { throw new Error(product); } } as unknown as FocusPreparedClient;
  const value = await loadPreparedFocusDetail(client, source, '1', 'opaque/tag', displayed, { ordinal: 0, token: 'current', completedByHour: 100 }, new AbortController().signal);
  assert.equal(value?.artwork.description, 'Description at 100');
  assert.deepEqual(requests.map(item => [item.product, item.params.hour, item.params.mode, item.params.tokens]), [['focus-art', 100, 'text', ['current']]]);
});
test('unavailable focus diagnostics preserve producer reasons and exact missing counts', () => {
  const value = { schema: 6, firstHour: 100, lastHour: 102, reasons: [
    { table: 'Focus', reason: 'mod reader refused form: exact reason', missingRows: null },
    { table: 'CompletedFocuses', reason: null, missingRows: '9007199254740993' },
  ] };
  assert.deepEqual(validateUnavailableFocus(value, 101), ['Focus: mod reader refused form: exact reason', 'CompletedFocuses: 9007199254740993 missing rows']);
  assert.throws(() => validateUnavailableFocus(value, 103));
  assert.throws(() => validateUnavailableFocus({ ...value, schema: 5 }, 101));
  assert.throws(() => validateUnavailableFocus({ ...value, reasons: [{ table: 'Focus', reason: null, missingRows: null }] }, 101));
});
test('null before accepted coverage remains pending until the requested extent grows', async () => {
  let queries = 0, waits = 0;
  const client = { preparationStatus: async (request: any) => ({ ...request, state: 'preparing', sections: [
    { id: 'focus', state: 'ready', label: 'Focus', done: '1', total: '2', error: null }], backgroundEnabled: true, bytes: '0', error: null }),
    preparedQuery: async () => ++queries === 1 ? null : row,
  } as unknown as FocusPreparedClient;
  const answer = await loadPreparedFocusHour(client, source, '1', 'tag', 101, new AbortController().signal, true, async () => { waits++; });
  assert.equal(answer, row); assert.equal(queries, 2); assert.equal(waits, 0);
});
test('focus hour polling stops an instant wait when a ready prefix does not grow', async () => {
  let queries = 0, statuses = 0, waits = 0;
  const client = {
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 3, 'stationary focus coverage must not request a fourth status');
      return { ...request, state: 'preparing', sections: [{ id: 'focus', state: 'ready', done: '1', total: '2' }],
        backgroundEnabled: true, bytes: String(statuses), error: null };
    },
    preparedQuery: async () => { assert.ok(++queries <= 3); return null; },
  } as unknown as FocusPreparedClient;
  await assert.rejects(loadPreparedFocusHour(client, source, '1', 'tag', 101, new AbortController().signal, false,
    async () => { waits++; }), { message: 'Focus preparation made no progress.' });
  assert.deepEqual({ queries, statuses, waits }, { queries: 3, statuses: 3, waits: 2 });
});
test('pending focus hour keeps paced polling through an unchanged ready prefix', async () => {
  let queries = 0, statuses = 0, waits = 0;
  const client = {
    preparationStatus: async (request: object) => {
      assert.ok(++statuses <= 3);
      return { ...request, state: 'preparing', sections: [{ id: 'focus', state: 'ready', done: '1', total: '2' }], backgroundEnabled: true, bytes: '0', error: null };
    },
    preparedQuery: async () => { assert.ok(++queries <= 4); return queries < 4 ? null : row; },
  } as unknown as FocusPreparedClient;
  assert.equal(await loadPreparedFocusHour(client, source, '1', 'tag', 101, new AbortController().signal, false,
    async () => { waits++; await new Promise(resolve => setTimeout(resolve, 500)); }), row);
  assert.deepEqual({ queries, statuses, waits }, { queries: 4, statuses: 3, waits: 2 });
});
test('focus hour polling accepts a growing prefix and stops after cancellation', async () => {
  let queries = 0, statuses = 0, waits = 0;
  const controller = new AbortController();
  const client = {
    preparationStatus: async (request: object) => ({ ...request, state: 'preparing', sections: [
      { id: 'focus', state: 'ready', done: String(++statuses), total: '5' }], backgroundEnabled: true, bytes: '0', error: null }),
    preparedQuery: async () => ++queries === 4 ? row : null,
  } as unknown as FocusPreparedClient;
  assert.equal(await loadPreparedFocusHour(client, source, '1', 'tag', 101, controller.signal, false, async () => { waits++; }), row);
  assert.deepEqual({ queries, statuses, waits }, { queries: 4, statuses: 3, waits: 2 });
  queries = 0; statuses = 0; waits = 0;
  await assert.rejects(loadPreparedFocusHour(client, source, '1', 'tag', 101, controller.signal, false,
    async () => { waits++; controller.abort(); }), { name: 'AbortError' });
  assert.deepEqual({ queries, statuses, waits }, { queries: 2, statuses: 2, waits: 1 });
});
test('focus readiness and pending hour replies cannot complete after cancellation', async () => {
  const statusController = new AbortController();
  const statusClient = { preparationStatus: async (request: object) => {
    statusController.abort(); return { ...request, state: 'ready', sections: [] };
  } } as unknown as FocusPreparedClient;
  await assert.rejects(waitForFocusPreparation(statusClient, source, statusController.signal), { name: 'AbortError' });
  let queries = 0, statuses = 0;
  const pointController = new AbortController();
  const pointClient = {
    preparationStatus: async (request: object) => { statuses++; return { ...request, state: 'ready', sections: [] }; },
    preparedQuery: async () => { if (++queries === 1) return null; pointController.abort(); return row; },
  } as unknown as FocusPreparedClient;
  await assert.rejects(loadPreparedFocusHour(pointClient, source, '1', 'tag', 101, pointController.signal), { name: 'AbortError' });
  assert.deepEqual({ queries, statuses }, { queries: 2, statuses: 1 });
});
test('null after ready is genuinely missing coverage and stops polling', async () => {
  let queries = 0, waits = 0;
  const client = { preparationStatus: async (request: any) => ({ ...request, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null }),
    preparedQuery: async () => { queries++; return null; },
  } as unknown as FocusPreparedClient;
  assert.equal(await loadPreparedFocusHour(client, source, '1', 'tag', 101, new AbortController().signal, false, async () => { waits++; }), null);
  assert.equal(queries, 2); assert.equal(waits, 0);
});

test('an accepted focus hour reads immediately without a preparation status request', async () => {
  let queries = 0;
  const client = { preparedQuery: async () => { queries++; return row; },
    preparationStatus: async () => { throw new Error('accepted hours need no readiness probe'); },
  } as unknown as FocusPreparedClient;
  assert.equal(await loadPreparedFocusHour(client, source, '1', 'tag', 101, new AbortController().signal), row);
  assert.equal(queries, 1);
});


test('idle after Clear and disabled background never start without explicit retry', async () => {
  for (const backgroundEnabled of [true, false]) {
    let starts = 0;
    const client = {
      preparationStatus: async (request: any) => ({ ...request, backgroundEnabled, state: 'idle', sections: [], bytes: '0', error: null }),
      prepareSource: async () => { starts++; throw new Error('unexpected automatic preparation'); },
    } as unknown as FocusPreparedClient;
    await assert.rejects(waitForFocusPreparation(client, source, new AbortController().signal), /Prepare this recording|Background preparation is off/);
    assert.equal(starts, 0);
  }
});
