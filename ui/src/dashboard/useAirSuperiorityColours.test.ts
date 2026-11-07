import type { FactFailure } from './sharedFacts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { AirTimeline } from './airSuperiority';
import type { ChartTokens } from '../charts/tokens';
import { CAPACITY_REACHED, COUNTRY_COLOURS_OVER_BUDGET } from '../generated/dashboard.ts';
import type { CountryColourReply } from '../generated/dashboard';
import type { DashboardClient, ProductReply, ProductRequest } from './client';
import type { BoundTrack } from './model';
import { nameKey, readFactBatch } from './countryFacts.ts';
import { forgetProducts } from './products.ts';
import type { FactPlan } from './countryFacts';
import { airColourGroups, airColourPlan, colourHours, COLOURS } from './useAirSuperiorityColours.ts';
import type { AirColourSources } from './useAirSuperiorityColours';

const branch = { id: '1', sessionId: '1', firstHour: 0, lastHour: 47, main: true, fork: null, joins: [], sampleBoundary: 'Synthetic', gaps: [] } as BoundTrack['branch'];
const side = (key: string, tag: string) => ({ key, tag, faction: null, colourFaction: null, name: tag, leader: null, coBelligerents: 1 });
const reply: AirTimeline = { view: 'timeline', branchId: '1', available: true, firstHour: 0, lastHour: 47, days: [23, 47],
  sides: [side('w:1:101', 'Y'), side('w:2:102', 'X')],
  anchorReads: { 'w:1:101': ['X', 10], 'w:2:102': ['X', 30] },
  segments: { X: [{ firstHour: 0, lastHour: 23 }, { firstHour: 24, lastHour: 47 }], Y: [{ firstHour: 24, lastHour: 47 }] },
  rows: [], excluded: [], theatreReads: [], scale: 100000, dayHours: 24, badBelow: 40000, goodAbove: 60000, warnings: [] };

test('native coalition anchors preserve independent dates when another holder reuses the tag', () => {
  const plan = airColourPlan(reply, undefined, 47, null, branch);
  assert.deepEqual(plan.wanted, [{ tag: 'X', hour: 10 }, { tag: 'X', hour: 30 }]);
  assert.equal(plan.sources.get(47)?.sides.get('w:1:101'), nameKey(10, 'X'));
  assert.equal(plan.sources.get(47)?.sides.get('w:2:102'), nameKey(30, 'X'));
  const missing = airColourPlan({ ...reply, segments: { X: [{ firstHour: 24, lastHour: 47 }] } }, undefined, 47, null, branch);
  assert.equal(missing.sources.get(47)?.sides.has('w:1:101'), false);
});

test('sampled country colours use supplying reads rather than represented day endpoints', () => {
  const sampled: AirTimeline = { ...reply, sides: [side('c:X', 'X')], anchorReads: {}, sampleHours: [10, 10] };
  const plan = airColourPlan(sampled, undefined, 10, null, branch);
  assert.deepEqual([...plan.sources.keys()], [10]);
  assert.deepEqual(plan.wanted, [{ tag: 'X', hour: 10 }]);
});

test('no-majority faction colours use the established country read and stay unknown without it', () => {
  const faction = { ...side('f:7', 'Y'), faction: '7', leader: 'Y' };
  const known = airColourPlan({ ...reply, sides: [faction], anchorReads: { 'f:7': ['X', 10] } }, undefined, 47, null, branch);
  assert.deepEqual(known.wanted, [{ tag: 'X', hour: 10 }]);
  const unknown = airColourPlan({ ...reply, sides: [faction], anchorReads: {} }, undefined, 47, null, branch);
  assert.deepEqual(unknown.wanted, []);
});

test('colour groups are coloured again only where reads arrive, and alike hours share one group', () => {
  const plan = (entries: [number, [string, string][]][]) => new Map<number, AirColourSources>(entries.map(([at, sides]) => [at, { sides: new Map(sides) }]));
  const history = plan([[23, [['c:X', nameKey(23, 'X')]]], [47, [['c:X', nameKey(47, 'X')]]], [71, [['c:X', nameKey(71, 'X')]]]]);
  const style = (tag: string, color: [number, number, number]) => ({ tag, name: tag, color });
  const styles = new Map<string, unknown>([[nameKey(23, 'X'), style('X', [16, 32, 48])], [nameKey(47, 'X'), style('X', [16, 32, 48])], [nameKey(71, 'X'), style('X', [112, 128, 144])]]);
  const first = airColourGroups(history, new Map(), key => styles.get(key), '#000000', false);
  assert.equal(first.get(23)?.group, first.get(47)?.group);
  assert.equal(first.get(71)?.group.sides.get('c:X'), '#708090');
  // The current day's read at the history's last hour joins it there; other hours keep their groups.
  styles.set(nameKey(5, 'Y'), style('Y', [1, 2, 3]));
  const next = airColourGroups(history, plan([[71, [['w:1', nameKey(5, 'Y')]]]]), key => styles.get(key), '#000000', false, first);
  assert.equal(next.get(23), first.get(23));
  assert.equal(next.get(47), first.get(47));
  assert.deepEqual([...next.get(71)!.group.sides], [['c:X', '#708090'], ['w:1', '#010203']]);
});

test('colour runs answer each planned hour, leaving unread hours unread and uncoloured countries without a value', () => {
  const reply: CountryColourReply = { branchId: '1', warnings: [], unread: [{ hour: 30, reason: 'not recorded' }], runs: [
    { tag: 'X', hour: 10, color: [1, 2, 3], missing: null }, { tag: 'Y', hour: 10, color: null, missing: 'not a country at this hour' },
    { tag: 'X', hour: 40, color: [4, 5, 6], missing: null }] };
  const hours = colourHours(reply, [[10, ['X', 'Y']], [20, ['X']], [30, ['X']], [40, ['X', 'Y']]]);
  assert.deepEqual([...hours.keys()], [10, 20, 40]);
  assert.deepEqual(hours.get(20)?.get('X')?.color, [1, 2, 3]);
  // One colour object per run, so alike hours compare as unchanged.
  assert.equal(hours.get(10)?.get('X'), hours.get(20)?.get('X'));
  assert.deepEqual(hours.get(40)?.get('X')?.color, [4, 5, 6]);
  assert.equal(hours.get(10)?.has('Y'), false);
  assert.equal(hours.get(40)?.has('Y'), false);
});

type Effect = { deps?: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
/** React's hooks for one component, rendered by hand: effects run on `commit`. */
function hooks() {
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  let cursor = 0;
  const same = (a?: readonly unknown[], b?: readonly unknown[]) => a && b && a.length === b.length && a.every((item, i) => Object.is(item, b[i]));
  const state = {
    begin() { cursor = 0; },
    useRef(initial: unknown) { const index = cursor++; if (!slots.has(index)) slots.set(index, { current: initial }); return slots.get(index); },
    useState(initial: unknown) {
      const index = cursor++; if (!slots.has(index)) slots.set(index, initial);
      return [slots.get(index), (next: unknown) => { slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next); }];
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, before = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (!before || !same(before.deps, deps)) slots.set(index, { deps, value: make() });
      return (slots.get(index) as { value: unknown }).value;
    },
    useEffect(run: Effect['run'], deps?: readonly unknown[]) {
      const index = cursor++; if (!same(effects.get(index)?.deps, deps)) pending.set(index, { run, deps });
    },
    commit() {
      for (const [index, effect] of pending) {
        effects.get(index)?.cleanup?.(); const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined; effects.set(index, effect);
      }
      pending.clear();
    },
    dispose() { for (const effect of effects.values()) effect.cleanup?.(); },
  };
  return state;
}

test('a long timeline reads every day’s colours in one request, which settles without asking again, an unread day muted', async () => {
  const host = globalThis as typeof globalThis & { __airColourHooks?: ReturnType<typeof hooks> };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'air-colour-reads', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (path.endsWith('/dashboard/countryNamesAt.ts') || path.endsWith('/dashboard/useAirSuperiorityColours.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, (_match, names: string) => names.split(',').map(name =>
            `const ${name.trim()} = (...args) => globalThis.__airColourHooks.${name.trim()}(...args);`).join('\n'));
      } }] });
  const harness = hooks();
  host.__airColourHooks = harness;
  try {
    const { useAirCountryColours } = await vite.ssrLoadModule('/ui/src/dashboard/useAirSuperiorityColours.ts') as typeof import('./useAirSuperiorityColours.ts');
    // Two years of daily cells, one country side.
    const days = Array.from({ length: 730 }, (_, day) => day * 24 + 23);
    const lastHour = days.at(-1)!;
    const track = { source: { id: 'synthetic', revision: '1' }, branch: { ...branch, lastHour } } as unknown as BoundTrack;
    const timeline: AirTimeline = { ...reply, lastHour, days, sides: [side('c:X', 'X')], anchorReads: {}, segments: { X: [{ firstHour: 0, lastHour }] } };
    const unread = days[5];
    const tokens = { surface: '#000000', countryColours: 'recorded', muted: '#777777' } as unknown as ChartTokens;
    const asked: { request: ProductRequest; answer: () => void }[] = [];
    const client = { product: (request: ProductRequest) => new Promise<ProductReply<unknown>>(resolve => {
      const data: CountryColourReply = { branchId: '1', runs: [{ tag: 'X', hour: 0, color: [16, 32, 48], missing: null }],
        unread: [{ hour: unread, reason: 'Hour not recorded' }], warnings: [] };
      asked.push({ request, answer: () => resolve({ ...request, data }) });
    }) } as unknown as DashboardClient;
    const draw = () => {
      harness.begin();
      const colours = useAirCountryColours(client, track, timeline, undefined, lastHour, null, undefined, true, tokens);
      harness.commit();
      return colours;
    };
    draw();
    // One request for every day, never one per day or per country.
    assert.deepEqual(asked.map(item => item.request.product), ['country-colours']);
    assert.equal((asked[0].request.params as { hours: number[] }).hours.length, days.length);
    assert.deepEqual((asked[0].request.params as { tags: string[] }).tags, ['X']);
    asked[0].answer();
    await new Promise<void>(resolve => setImmediate(resolve));
    for (let render = 0; render < 3; render++) {
      const colours = draw();
      assert.ok(days.every(day => day === unread || colours.reply.at(day)?.sides.get('c:X') === '#102030'));
      // The day the reply could not read is drawn muted, and not asked for again.
      assert.equal(colours.reply.color(side('c:X', 'X'), unread), '#777777');
    }
    // Answered once, nothing is asked again and nothing stays open.
    assert.equal(asked.length, 1);
  } finally { harness.dispose(); delete host.__airColourHooks; await vite.close(); }
});

/** A client answering `country-colours` with one run per country (refusing more than
 * `most` hours as too large) and `country-styles` with one colour, counting requests. */
function colourClient(most = Infinity) {
  const asked: { product: string; hours: number[] }[] = [];
  const limit = { most };
  const client = { product: async (request: ProductRequest) => {
    const params = request.params as { hours?: number[]; hour?: number; tags: string[] };
    asked.push({ product: request.product, hours: params.hours ?? [params.hour!] });
    if (request.product === 'country-styles')
      return { ...request, data: { branchId: '1', hour: params.hour, warnings: [], styles: params.tags.map(tag => ({ tag, name: tag, color: [1, 2, 3] })) } };
    if (params.hours!.length * params.tags.length > limit.most) throw new Error(`${COUNTRY_COLOURS_OVER_BUDGET} (64 bytes of JSON); ask for fewer hours`);
    const data: CountryColourReply = { branchId: '1', warnings: [], unread: [],
      runs: params.tags.map(tag => ({ tag, hour: Math.min(...params.hours!), color: [4, 5, 6], missing: null })) };
    return { ...request, data };
  } } as unknown as DashboardClient;
  return { client, asked, limit };
}
/** Reads `plan` of the recording `sourceId` (each test its own, so none shares a size another found). */
async function readColours(client: DashboardClient, plan: FactPlan, sourceId: string, signal = new AbortController().signal) {
  const told: [number, unknown, boolean | undefined][] = [];
  await readFactBatch(client, { sourceId, revision: '1', branchId: '1' }, COLOURS, plan, signal, (hour, _tags, found, retryable) =>
    told.push([hour, found ? [...found.values()].map(value => (value as { color: unknown }).color) : null, retryable]));
  return told;
}

test('a reply too large for one request is read again in halves until every hour is coloured', async () => {
  const { client, asked } = colourClient(2);
  const plan: FactPlan = [10, 20, 30, 40, 50].map(hour => [hour, ['X']] as const);
  const told = await readColours(client, plan, 'halves');
  assert.deepEqual(told.map(([hour]) => hour), [10, 20, 30, 40, 50]);
  assert.ok(told.every(([, found]) => found !== null));
  assert.deepEqual(asked.map(item => item.hours), [[10, 20, 30, 40, 50], [10, 20], [30, 40, 50], [30], [40, 50]]);
  // A single hour is read as one hour's styles, on the interactive path.
  assert.equal(asked[3].product, 'country-styles');
});

test('one hour is read through country-styles, as a day Follow adds always was', async () => {
  const { client, asked } = colourClient();
  const told = await readColours(client, [[70, ['X', 'Y']]], 'single');
  assert.deepEqual(asked, [{ product: 'country-styles', hours: [70] }]);
  assert.deepEqual(told, [[70, [[1, 2, 3], [1, 2, 3]], undefined]]);
});

test('a reply that cannot be understood fails without retrying, and one after abort tells nothing', async () => {
  const malformed = { product: async (request: ProductRequest) => ({ ...request, data: { runs: null } }) } as unknown as DashboardClient;
  assert.deepEqual(await readColours(malformed, [[10, ['X']], [20, ['X']]], 'malformed'), [[10, null, false], [20, null, false]]);
  const controller = new AbortController();
  const late = { product: async (request: ProductRequest) => { controller.abort(); return { ...request, data: { branchId: '1', runs: [], unread: [], warnings: [] } }; } } as unknown as DashboardClient;
  assert.deepEqual(await readColours(late, [[10, ['X']], [20, ['X']]], 'late', controller.signal), []);
});

test('after a refusal, the recording’s next plan starts at the most hours one reply held', async () => {
  const { client, asked } = colourClient(2);
  const plan: FactPlan = [10, 20, 30, 40, 50].map(hour => [hour, ['X']] as const);
  await readColours(client, plan, 'remount');
  asked.length = 0;
  const told = await readColours(client, plan, 'remount');
  assert.ok(told.every(([, found]) => found !== null));
  assert.deepEqual(asked.map(item => item.hours), [[10, 20], [30, 40], [50]]);
});

test('one hour read in pieces keeps the countries of the pieces that answered', async () => {
  const tags = Array.from({ length: 70 }, (_, index) => `T${index}`);
  const client = { product: async (request: ProductRequest) => {
    const params = request.params as { hour: number; tags: string[] };
    if (params.tags.includes('T64')) throw new Error('Country styles unavailable at this hour: synthetic');
    return { ...request, data: { branchId: '1', hour: params.hour, warnings: [], styles: params.tags.map(tag => ({ tag, name: tag, color: [1, 2, 3] })) } };
  } } as unknown as DashboardClient;
  const told: { found: number | null; retryable?: boolean; failed?: ReadonlyMap<string, FactFailure> }[] = [];
  await readFactBatch(client, { sourceId: 'pieces', revision: '1', branchId: '1' }, COLOURS, [[70, tags]], new AbortController().signal,
    (_hour, _tags, found, retryable, failed) => told.push({ found: found?.size ?? null, retryable, failed }));
  assert.deepEqual(told.map(item => ({ ...item, failed: item.failed && new Map([...item.failed].map(([tag, failure]) => [tag, failure.retryable])) })), [{ found: 64, retryable: undefined, failed: new Map(tags.slice(64).map(tag => [tag, false])) }]);
});

test('each failed piece of an hour keeps its own retry, so a refusal for want of room is tried again', async () => {
  const tags = Array.from({ length: 150 }, (_, index) => `T${index}`);
  const client = { product: async (request: ProductRequest) => {
    const params = request.params as { hour: number; tags: string[] };
    if (params.tags.includes('T64')) throw new Error(`${CAPACITY_REACHED} (synthetic)`);
    if (params.tags.includes('T128')) throw new Error('Country styles unavailable at this hour: synthetic');
    return { ...request, data: { branchId: '1', hour: params.hour, warnings: [], styles: params.tags.map(tag => ({ tag, name: tag, color: [1, 2, 3] })) } };
  } } as unknown as DashboardClient;
  let failed: ReadonlyMap<string, FactFailure> | undefined;
  await readFactBatch(client, { sourceId: 'retries', revision: '1', branchId: '1' }, COLOURS, [[70, tags]], new AbortController().signal,
    (_hour, _tags, _found, _retryable, unanswered) => { failed = unanswered; });
  assert.deepEqual(failed && new Map([...failed].map(([tag, failure]) => [tag, failure.retryable])), new Map([...tags.slice(64, 128).map(tag => [tag, true] as const), ...tags.slice(128).map(tag => [tag, false] as const)]));
});

test('a refusal at the remembered size lowers it', async () => {
  const { client, asked, limit } = colourClient(2);
  const plan: FactPlan = [10, 20, 30, 40, 50].map(hour => [hour, ['X']] as const);
  await readColours(client, plan, 'shrinking');
  // The recording's replies grow: two hours no longer fit one reply.
  limit.most = 1;
  await readColours(client, plan, 'shrinking');
  asked.length = 0;
  const told = await readColours(client, plan, 'shrinking');
  assert.ok(told.every(([, found]) => found !== null));
  assert.ok(asked.every(item => item.hours.length === 1), JSON.stringify(asked));
});

test('the size kept is the request’s own: every hour named with every country named', async () => {
  // Each hour wants another country, but one request reads both at both hours.
  const { client, asked } = colourClient(4);
  const plan: FactPlan = [[10, ['A']], [20, ['B']], [30, ['A']], [40, ['B']]];
  await readColours(client, plan, 'crossed');
  asked.length = 0;
  const told = await readColours(client, plan, 'crossed');
  assert.ok(told.every(([, found]) => found !== null));
  // Two hours of A and B are four countries read: the most one reply held.
  assert.deepEqual(asked.map(item => item.hours), [[10, 20], [30, 40]]);
});

test('a recording released forgets the size its replies were found to fit', async () => {
  const { client, asked } = colourClient(2);
  const plan: FactPlan = [10, 20, 30, 40, 50].map(hour => [hour, ['X']] as const);
  await readColours(client, plan, 'released');
  forgetProducts(client, 'released');
  asked.length = 0;
  await readColours(client, plan, 'released');
  assert.deepEqual(asked[0].hours, [10, 20, 30, 40, 50]);
});
