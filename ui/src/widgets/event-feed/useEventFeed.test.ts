import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient } from '../../dashboard/client';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { EventFeedReply } from '../../generated/dashboard';
import type { FeedFilter, PreparedFeedPage } from './eventFeedData';

test('actual feed hook isolates filtered counts and wanted pages across identical-source clients', async () => {
  type Effect = { run: () => void; deps: readonly unknown[] };
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  const inputs = new Map<DashboardClient, { manifest?: PreparedFeedPage; chunks: ReadonlyMap<number, PreparedFeedPage> }>();
  let cursor = 0;
  const same = (a: readonly unknown[], b: readonly unknown[]) => a.length === b.length && a.every((value, i) => Object.is(value, b[i]));
  const hooks = {
    inputs, wanted: [] as readonly number[], requests: [] as unknown[], first: undefined as unknown,
    useState(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, initial);
      return [slots.get(index), (next: unknown) => slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next)];
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, { current: initial });
      return slots.get(index);
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, old = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (!old || !same(old.deps, deps)) slots.set(index, { deps: [...deps], value: make() });
      return (slots.get(index) as { value: unknown }).value;
    },
    useCallback(callback: unknown, deps: readonly unknown[]) { return hooks.useMemo(() => callback, deps); },
    useEffect(run: () => void, deps: readonly unknown[]) {
      const index = cursor++, old = effects.get(index);
      if (!old || !same(old.deps, deps)) pending.set(index, { run, deps: [...deps] });
    },
    useLayoutEffect(run: () => void, deps: readonly unknown[]) { hooks.useEffect(run, deps); },
    commit() { for (const [index, effect] of pending) { effect.run(); effects.set(index, effect); } pending.clear(); },
  };
  const host = globalThis as typeof globalThis & { __eventFeedHooks?: typeof hooks };
  host.__eventFeedHooks = hooks;
  // Only the dispatcher and authenticated product boundaries are synthetic; preparation,
  // country filtering, playback cuts and page/count bookkeeping use the actual hook source.
  const vite = await createServer({ configFile: false, mode: 'parity', optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'event-feed-client-fixture', enforce: 'pre',
      transform(source, id) {
        if (!id.replaceAll('\\', '/').endsWith('/widgets/event-feed/useEventFeed.ts')) return;
        source = source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__eventFeedHooks;');
        for (const path of ['../../dashboard/contentRevision', '../../dashboard/productWindows', '../../dashboard/products'])
          source = source.replace(`from '${path}'`, "from 'event-feed-inputs'");
        return source;
      },
      resolveId(id) { if (id === 'event-feed-inputs') return '\0event-feed-inputs'; },
      load(id) { if (id === '\0event-feed-inputs') return `
        export const useContentRevision = () => 0;
        // The first page answers the hour it was asked for at once.
        export const useFollowedProduct = (client, request, moving, active, validate) => {
          globalThis.__eventFeedHooks.first = request;
          const data = globalThis.__eventFeedHooks.inputs.get(client)?.manifest;
          return { data: data && validate(data), moment: data && request ? JSON.stringify(moving.map(name => request.params[name])) : undefined, retry() {} };
        };
        const empty = new Map();
        export const useProductWindows = (client, identity, request, wanted, active, validate) => {
          globalThis.__eventFeedHooks.wanted = wanted;
          globalThis.__eventFeedHooks.requests = wanted.map(request);
          const chunks = globalThis.__eventFeedHooks.inputs.get(client)?.chunks ?? empty;
          return { data: new Map([...chunks].map(([key, page]) => [key, validate(page)])), errors: empty, retry() {} };
        };`; },
    }] });
  try {
    const { useEventFeed } = await vite.ssrLoadModule('/ui/src/widgets/event-feed/useEventFeed.ts') as typeof import('./useEventFeed');
    const track = { source: { id: 'same-source', revision: 'same-revision' }, branch: { id: '7', firstHour: 100, lastHour: 400 } } as BoundTrack;
    const a = {} as DashboardClient, b = {} as DashboardClient;
    const filter: FeedFilter = { categories: new Set(['milestones']), country: tag => tag === 'selected', firstHour: null };
    const reply = (rows: EventFeedReply['rows'] = []): EventFeedReply => ({ branchId: '7', firstHour: 100, lastHour: 400,
      windows: [], rows, items: [], modifiers: [], texts: [], unavailable: [], warnings: [] });
    const event = (id: string, hour: number, tag = 'selected') => ({ id, kind: 'focusCompleted' as const, category: 'milestones' as const,
      hour, readHour: hour, nameHour: hour, exact: false, tags: [tag], item: null, amount: null });
    const first: PreparedFeedPage = { reply: reply([event('newest', 350, 'unselected')]), nextCursor: 'keeps-live-400-native-350' };
    const older: PreparedFeedPage = { reply: reply([event('older', 250)]), nextCursor: 'keeps-live-400-native-250' };
    const late: PreparedFeedPage = { reply: reply([{ ...event('late', 125), readHour: 390 }]), nextCursor: null };
    inputs.set(a, { manifest: first, chunks: new Map() }); inputs.set(b, { chunks: new Map() });
    const render = (client: DashboardClient, chosen = filter, prepareAll = false, hour = 400) => { cursor = 0;
      return useEventFeed({ client } as WidgetRuntime, track, true, hour, chosen, 'same-filter', prepareAll); };
    let feed = render(a); hooks.commit(); feed = render(a); hooks.commit();
    assert.equal(feed.pages[0].count, 0, 'an empty filtered first page stays complete');
    assert.equal(feed.pages[1].rows, undefined, 'a continuation keeps older matches reachable');
    feed.onWantedPages(['1']); render(a);
    assert.deepEqual(hooks.wanted, [1]);
    assert.deepEqual((hooks.requests[0] as { params: unknown }).params, { branchId: '7', firstHour: null, lastHour: 400,
      categories: ['milestones'], observations: false, cursor: 'keeps-live-400-native-350', pageSize: 256 });
    const previousWanted = feed.onWantedPages;
    assert.deepEqual(render(b).pages, [], 'loading another client masks all held rows');
    inputs.set(b, { manifest: first, chunks: new Map() });
    feed = render(b); hooks.commit(); feed = render(b); hooks.commit();
    feed.onWantedPages(['1']); previousWanted(['0']); render(b);
    assert.deepEqual(hooks.wanted, [1], 'departed callbacks cannot change current demand');
    inputs.set(b, { manifest: first, chunks: new Map([[1, older]]) });
    feed = render(b); hooks.commit(); feed = render(b); hooks.commit();
    assert.equal(feed.pages.length, 3);
    assert.deepEqual(feed.pages.flatMap(page => page.rows ?? []).map(row => row.item.event.id), ['older']);
    feed = render(b, filter, true); hooks.commit(); render(b, filter, true);
    assert.deepEqual(hooks.wanted, [1, 2], 'parity demands every discovered continuation');
    inputs.set(b, { manifest: first, chunks: new Map([[1, older], [2, late]]) });
    feed = render(b, filter, true); hooks.commit(); feed = render(b, filter, true);
    assert.ok(feed.pages.every(page => page.rows !== undefined));
    assert.deepEqual(feed.pages.flatMap(page => page.rows ?? []).map(row => [row.item.event.id, row.revealedHour]), [['older', 250], ['late', 390]]);
    const ranged: FeedFilter = { ...filter, firstHour: 200 };
    feed = render(b, ranged, true); hooks.commit(); feed = render(b, ranged, true);
    assert.deepEqual(feed.pages.flatMap(page => page.rows ?? []).map(row => row.item.event.id), ['older']);
    render(b, filter, true, 350); hooks.commit(); render(b, filter, true, 350); hooks.commit();
    assert.deepEqual(render(b, filter, true, 350).pages.flatMap(page => page.rows ?? []).map(row => row.item.event.id), ['older'], 'a supplying read after the Live bound stays hidden');
    const native = { ...event('native-repeat', 125), readHour: 350, kind: 'warDeclared' as const, category: 'warsPeace' as const,
      exact: true, tags: ['selected', 'target'], participants: ['selected', 'target'] as [string, string] };
    inputs.set(b, { manifest: { reply: reply([native]), nextCursor: 'next-native-duplicate' }, chunks: new Map([[1, { reply: reply([native]), nextCursor: null }]]) });
    const warFilter: FeedFilter = { ...filter, categories: new Set(['warsPeace']) };
    render(b, warFilter, true); hooks.commit(); render(b, warFilter, true); hooks.commit();
    const duplicateRows = render(b, warFilter, true).pages.flatMap(page => page.rows ?? []);
    assert.equal(duplicateRows.length, 2);
    assert.notEqual(duplicateRows[0].key, duplicateRows[1].key, 'identical native ledger rows on different pages keep separate log owners');
    assert.deepEqual(duplicateRows.map(row => row.item.event.id), ['native-repeat', 'native-repeat']);
    const many = Array.from({ length: 5000 }, (_, index) => event(`prepared:focus:${index}`, 350));
    let manyChunks = new Map<number, PreparedFeedPage>();
    inputs.set(a, { manifest: { reply: reply(many.slice(0, 256)), nextCursor: 'page-1' }, chunks: manyChunks });
    const requestedPages = new Set<number>();
    for (let turn = 0; turn < 2 * Math.ceil(5000 / 256) + 2; turn++) {
      feed = render(a, filter, true, 399); hooks.commit();
      for (const index of hooks.wanted) {
        if (requestedPages.has(index)) continue;
        requestedPages.add(index);
        assert.ok(requestedPages.size <= Math.ceil(5000 / 256) - 1, '5000 rows issue only their nineteen continuation requests');
        assert.equal((hooks.requests[hooks.wanted.indexOf(index)] as { params: { cursor: string } }).params.cursor, `page-${index}`);
        manyChunks = new Map(manyChunks).set(index, { reply: reply(many.slice(index * 256, (index + 1) * 256)),
          nextCursor: (index + 1) * 256 < many.length ? `page-${index + 1}` : null });
        inputs.set(a, { ...inputs.get(a)!, chunks: manyChunks });
      }
    }
    feed = render(a, filter, true, 399); hooks.commit();
    assert.equal(feed.pages.flatMap(page => page.rows ?? []).length, 5000);
    assert.equal(feed.pages.length, Math.ceil(5000 / 256));
    assert.equal(requestedPages.size, 19);
    const obsolete: PreparedFeedPage = { reply: reply(), nextCursor: 'obsolete-1' };
    const project: PreparedFeedPage = { reply: reply([{ ...event('current-project', 200), kind: 'projectFinished' }]), nextCursor: null };
    const obsoleteChunks = new Map<number, PreparedFeedPage>();
    inputs.set(a, { manifest: obsolete, chunks: obsoleteChunks });
    const afterObsolete = new Set<number>();
    for (let turn = 0; turn < 8; turn++) {
      feed = render(a, filter, true, 397); hooks.commit();
      assert.equal(feed.error, undefined, 'a raw page removed by Live validation still advances');
      for (const index of hooks.wanted) {
        if (afterObsolete.has(index)) continue;
        afterObsolete.add(index);
        assert.ok(afterObsolete.size <= 2, 'two obsolete pages reach the live row in three total requests');
        assert.equal((hooks.requests[hooks.wanted.indexOf(index)] as { params: { cursor: string } }).params.cursor, `obsolete-${index}`);
        obsoleteChunks.set(index, index === 1 ? { reply: reply(), nextCursor: 'obsolete-2' } : project);
        inputs.set(a, { manifest: obsolete, chunks: new Map(obsoleteChunks) });
      }
    }
    feed = render(a, filter, true, 397); hooks.commit();
    assert.equal(feed.pages.length, 3);
    assert.deepEqual(feed.pages.map(page => page.count), [0, 0, 1]);
    assert.deepEqual(feed.pages.flatMap(page => page.rows ?? []).map(row => row.item.event.id), ['current-project']);
    assert.equal(afterObsolete.size, 2);
    inputs.set(a, { manifest: { reply: reply(), nextCursor: null }, chunks: new Map() });
    render(a, filter, true, 398); hooks.commit(); feed = render(a, filter, true, 398); hooks.commit();
    assert.equal(feed.pages.length, 1, 'an empty terminal page ends discovery');
    assert.deepEqual(hooks.wanted, []);
    const stalled: PreparedFeedPage = { reply: reply(), nextCursor: first.nextCursor };
    inputs.set(a, { manifest: first, chunks: new Map([[1, stalled]]) });
    render(a, filter, true); hooks.commit();
    for (let repeat = 0; repeat < 8; repeat++) {
      feed = render(a, filter, true); hooks.commit();
      assert.ok(feed.pages.length <= 2, 'a repeated cursor never grows the discovery chain');
      assert.ok(hooks.requests.length <= 1, 'a stalled continuation never requests another page');
    }
    assert.match(feed.error!, /continuation made no progress/);
    inputs.set(a, { manifest: first, chunks: new Map([[1, { ...stalled, nextCursor: null }]]) });
    render(a, filter, true); hooks.commit();
    assert.equal(render(a, filter, true).error, undefined, 'a progressed reply clears the refusal');
    inputs.set(b, { manifest: undefined, chunks: new Map() });
    assert.deepEqual(render(b, filter, false, 399).pages, [], 'a changed Live bound starts a fresh cursor owner');
    render(b, { ...filter, observations: true }, false, 399);
    assert.equal((hooks.first as { params: { observations: boolean } }).params.observations, true, 'shown recorder observations are asked for');
  } finally { delete host.__eventFeedHooks; await vite.close(); }
});
