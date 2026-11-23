import assert from 'node:assert/strict';
import test from 'node:test';
import { factsKey, HourStore, nameKey, planRequests, readFacts, wantedHours } from './countryFacts.ts';
import type { FactSpec } from './countryFacts';
import { loadProduct } from './products.ts';
import type { DashboardClient, ProductRequest } from './client';
import { productRequest } from './request.ts';

const found = (entries: [string, string][]) => new Map(entries);

test('the store keeps only its most recently used hours and hands out only what is asked', () => {
  const store = new HourStore<string>(2);
  store.record(1, ['GER', 'ITA'], found([['GER', 'German Reich']]));
  store.record(2, ['GER'], found([['GER', 'Germany']]));
  assert.equal(store.answered(1, 'ITA'), true);
  assert.equal(store.get(1, 'ITA'), undefined);
  store.touch([1]);
  store.record(3, ['GER'], found([['GER', 'Third Reich']]));
  assert.equal(store.size, 2);
  assert.equal(store.get(2, 'GER'), undefined);
  assert.equal(store.answered(2, 'GER'), false);
  assert.deepEqual([...store.pick([{ hour: 1, tag: 'GER' }, { hour: 2, tag: 'GER' }])], [[nameKey(1, 'GER'), 'German Reich']]);
  store.clear();
  assert.equal(store.size, 0);
});

test('following playback with a tooltip open holds a bounded number of hours', () => {
  const store = new HourStore<string>(64);
  const tags = Array.from({ length: 32 }, (_, index) => `T${index}`);
  for (let hour = 0; hour < 500; hour++) store.record(hour, tags, new Map(tags.map(tag => [tag, `${tag}@${hour}`])));
  assert.equal(store.size, 64);
  assert.equal(store.get(499, 'T0'), 'T0@499');
  assert.equal(store.get(435, 'T0'), undefined);
  assert.equal(store.pick(tags.map(tag => ({ hour: 499, tag }))).size, 32);
});

test('hours once wanted together stay held when fewer are wanted, the oldest dropped past that', () => {
  const store = new HourStore<string>(2);
  const hours = Array.from({ length: 10 }, (_, hour) => hour);
  store.want(hours);
  for (const hour of hours) store.record(hour, ['GER'], found([['GER', `GER@${hour}`]]));
  store.want([0]);
  assert.equal(store.size, 10);
  assert.deepEqual(planRequests(store, hours.map(hour => ({ hour, tag: 'GER' })), Infinity, 64), []);
  store.record(10, ['GER'], found([['GER', 'GER@10']]));
  assert.equal(store.size, 11);
  store.record(11, ['GER'], found([['GER', 'GER@11']]));
  assert.equal(store.size, 11);
  assert.equal(store.answered(1, 'GER'), false);
  assert.equal(store.get(0, 'GER'), 'GER@0');
});

test('requests skip answered countries, keep the earliest hours and split large hours', () => {
  const store = new HourStore<string>(8);
  store.record(10, ['GER'], found([['GER', 'x']]));
  const wanted = [{ hour: 10, tag: 'GER' }, { hour: 10, tag: 'ITA' }, { hour: 30, tag: 'A' }, { hour: 30, tag: 'B' }, { hour: 30, tag: 'C' }, { hour: 20, tag: 'SOV' }];
  assert.deepEqual(planRequests(store, wanted, 8, 2), [[10, ['ITA']], [20, ['SOV']], [30, ['A', 'B']], [30, ['C']]]);
  assert.deepEqual(planRequests(store, wanted, 2, 64), [[10, ['ITA']], [20, ['SOV']]]);
});

test('artwork list order loads the first rows before earlier dated entries without omitting any requested token', () => {
  const store = new HourStore<string>(8);
  const wanted = [{ hour: 30, tag: 'Z' }, { hour: 30, tag: 'B' }, { hour: 30, tag: 'A' },
    { hour: 10, tag: 'old completion' }, { hour: 30, tag: 'Z' }];
  assert.deepEqual(planRequests(store, wanted, Infinity, 2, 'wanted'),
    [[30, ['Z', 'B']], [30, ['A']], [10, ['old completion']]]);
  store.record(30, ['Z'], found([['Z', 'visible title']]));
  assert.deepEqual(planRequests(store, wanted, Infinity, 2, 'wanted'),
    [[30, ['B', 'A']], [10, ['old completion']]]);
  assert.deepEqual(planRequests(store, wanted, 1, 2, 'wanted'), [[10, ['old completion']]]);
  const reversed = [...wanted].reverse();
  assert.equal(factsKey('track', wanted), factsKey('track', reversed));
  assert.notEqual(factsKey('track', wanted, 'wanted'), factsKey('track', reversed, 'wanted'));
});

test('wanting more hours than the cap keeps them all, so requests stop once each is answered', () => {
  const store = new HourStore<string>(64);
  const wanted = Array.from({ length: 100 }, (_, hour) => ({ hour, tag: 'GER' }));
  let requests = 0;
  for (let render = 0; render < 10; render++) {
    store.want(wantedHours(wanted, Infinity));
    const plan = planRequests(store, wanted, Infinity, 64);
    requests += plan.length;
    for (const [hour, tags] of plan) store.record(hour, tags, new Map(tags.map(tag => [tag, `${tag}@${hour}`])));
  }
  assert.equal(requests, 100);
  assert.equal(store.pick(wanted).size, 100);
  for (let hour = 1000; hour < 1200; hour++) store.record(hour, ['ITA'], new Map([['ITA', 'x']]));
  assert.equal(planRequests(store, wanted, Infinity, 64).length, 0);
  assert.equal(store.size, 200);
  store.want([]);
  assert.equal(store.size, 100);
});

test('per-hour reads go around the shared product cache, so they never evict widget data', async () => {
  const calls = new Map<string, number>();
  const client = { product: async (input: ProductRequest) => {
    calls.set(input.product, (calls.get(input.product) ?? 0) + 1);
    if ((input.params as { hour?: number }).hour === -1) throw new Error('gap');
    return { sourceId: input.sourceId, revision: input.revision, product: input.product, data: { names: input.params } };
  } } as unknown as DashboardClient;
  const history: ProductRequest = productRequest({ sourceId: 's', revision: 'v', product: 'resource-balance', params: { branchId: '1', tag: 'GER' } });
  await loadProduct(client, history, new AbortController().signal);
  const spec: FactSpec<string> = { product: 'country-styles', perRequest: 64, params: (branchId, hour, tags) => ({ branchId, hour, tags }),
    extract: data => new Map(((data as { names: { tags: string[] } }).names.tags).map(tag => [tag, `${tag}!`])) };
  const seen: [number, number | null][] = [];
  const plan = Array.from({ length: 300 }, (_, hour) => [hour, ['GER']] as const);
  await readFacts(client, { sourceId: 's', revision: 'v', branchId: '1' }, spec, [...plan, [-1, ['GER']]], new AbortController().signal,
    (hour, _tags, values) => seen.push([hour, values ? values.size : null]));
  assert.equal(calls.get('country-styles'), 301);
  assert.deepEqual(seen.at(-1), [-1, null]);
  assert.deepEqual(seen[0], [0, 1]);
  await loadProduct(client, history, new AbortController().signal);
  assert.equal(calls.get('resource-balance'), 1);
  await readFacts(client, { sourceId: 's', revision: 'v', branchId: '1' }, spec, [[0, ['GER']]], new AbortController().signal, () => {});
  assert.equal(calls.get('country-styles'), 302);
});

test('a shared spec asks a country at an hour once however many reads want it, an unshared one each time', async () => {
  const asked: string[][] = [];
  const client = { product: async (input: ProductRequest) => {
    const tags = (input.params as { tags: string[] }).tags;
    asked.push(tags);
    return { sourceId: input.sourceId, revision: input.revision, product: input.product, data: { tags } };
  } } as unknown as DashboardClient;
  const own: FactSpec<string> = { product: 'country-styles', perRequest: 64, params: (branchId, hour, tags) => ({ branchId, hour, tags }),
    extract: data => new Map((data as { tags: string[] }).tags.map(tag => [tag, `${tag}!`])) };
  const shared: FactSpec<string> = { ...own, shared: true };
  const source = { sourceId: 's', revision: 'v', branchId: '1' };
  for (const [spec, expected] of [[shared, [['GER', 'ITA'], ['JAP']]], [own, [['GER', 'ITA'], ['ITA', 'JAP']]]] as const) {
    asked.length = 0;
    const told: string[] = [];
    const tell = (_hour: number, tags: readonly string[], found: ReadonlyMap<string, string> | null) => {
      for (const tag of tags) told.push(`${tag}=${found?.get(tag)}`);
    };
    await Promise.all([
      readFacts(client, source, spec, [[0, ['GER', 'ITA']]], new AbortController().signal, tell),
      readFacts(client, source, spec, [[0, ['ITA', 'JAP']]], new AbortController().signal, tell),
    ]);
    assert.deepEqual(asked, expected);
    assert.deepEqual(told.sort(), ['GER=GER!', 'ITA=ITA!', 'ITA=ITA!', 'JAP=JAP!']);
  }
});
