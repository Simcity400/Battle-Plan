import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient } from '../../dashboard/client';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { EventFeedReply } from '../../generated/dashboard';
import type { FeedFilter } from './eventFeedData';

test('actual feed hook isolates filtered counts and wanted pages across identical-source clients', async () => {
  type Effect = { run: () => void; deps: readonly unknown[] };
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  const inputs = new Map<DashboardClient, { manifest?: EventFeedReply; chunks: ReadonlyMap<number, EventFeedReply> }>();
  let cursor = 0;
  const same = (a: readonly unknown[], b: readonly unknown[]) => a.length === b.length && a.every((value, i) => Object.is(value, b[i]));
  const hooks = {
    inputs, wanted: [] as readonly number[],
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
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
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
        export const useProduct = client => ({ data: globalThis.__eventFeedHooks.inputs.get(client)?.manifest, retry() {} });
        const empty = new Map();
        export const useProductWindows = (client, identity, request, wanted) => {
          globalThis.__eventFeedHooks.wanted = wanted;
          return { data: globalThis.__eventFeedHooks.inputs.get(client)?.chunks ?? empty, errors: empty, retry() {} };
        };`; },
    }] });
  try {
    const { useEventFeed } = await vite.ssrLoadModule('/ui/src/widgets/event-feed/useEventFeed.ts') as typeof import('./useEventFeed');
    const track = { source: { id: 'same-source', revision: 'same-revision' }, branch: { id: '7', firstHour: 100, lastHour: 400 } } as BoundTrack;
    const a = {} as DashboardClient, b = {} as DashboardClient;
    const filter: FeedFilter = { categories: new Set(['milestones']), country: tag => tag === 'selected', firstHour: null };
    const manifest: EventFeedReply = { branchId: '7', firstHour: 100, lastHour: 400, windows: [100, 201].map(firstHour => ({
      firstHour, lastHour: firstHour === 100 ? 200 : 400, count: 5,
      categoryCounts: { warsPeace: 0, diplomacy: 0, politics: 0, milestones: 5 },
    })), rows: [], items: [], modifiers: [], texts: [], unavailable: [], warnings: [] };
    const page: EventFeedReply = { ...manifest, firstHour: 100, lastHour: 200, windows: [],
      rows: Array.from({ length: 5 }, (_, index) => ({ id: `unselected-focus-${index}`, kind: 'focusCompleted', category: 'milestones',
        hour: 120, readHour: 120, nameHour: 120, exact: false, tags: ['unselected'], item: null, amount: null })) };
    inputs.set(a, { manifest, chunks: new Map([[0, page]]) }); inputs.set(b, { chunks: new Map() });
    const render = (client: DashboardClient) => { cursor = 0; return useEventFeed({ client } as WidgetRuntime, track, true, 400, filter, 'same-filter', false); };
    let feed = render(a); feed.onWantedPages(['0']); hooks.commit(); feed = render(a); hooks.commit();
    assert.equal(feed.pages.find(page => page.key === '0')!.count, 0);
    assert.deepEqual(hooks.wanted, [0]);
    assert.equal(render(a).pages, feed.pages, 'unchanged same-client reads retain page references');
    inputs.set(a, { manifest, chunks: new Map() });
    feed = render(a); hooks.commit();
    assert.equal(feed.pages.find(page => page.key === '0')!.count, 0, 'completed counts survive same-client page eviction');
    const previousWanted = feed.onWantedPages;
    assert.deepEqual(render(b).pages, [], 'loading the new client masks the old manifest');
    inputs.set(b, { manifest, chunks: new Map() });
    feed = render(b); // The old client's effects and count state have not been cleaned up.
    assert.equal(feed.pages.find(page => page.key === '0')!.count, 5, 'an unloaded B page keeps B manifest capacity despite A filtered zero');
    assert.deepEqual(hooks.wanted, [1], 'the new owner starts with its visible newest page');
    hooks.commit(); feed.onWantedPages(['0', '1']);
    previousWanted(['0']);
    render(b);
    assert.deepEqual(hooks.wanted, [0, 1], 'an old callback cannot erase the current owner page selection');
    inputs.set(a, { manifest, chunks: new Map() });
    feed = render(a);
    assert.equal(feed.pages.find(page => page.key === '0')!.count, 5, 'A to B to A does not resurrect an obsolete filtered count');
    hooks.commit(); feed.onWantedPages(['1', '0']); previousWanted(['0']); render(a);
    assert.deepEqual(hooks.wanted, [1, 0], 'returning to A does not authorize its previous callback');
  } finally { delete host.__eventFeedHooks; await vite.close(); }
});
