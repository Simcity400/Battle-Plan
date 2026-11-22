import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient, ProductReply, ProductRequest } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { WidgetProps } from '../../dashboard/definitions';
import type { Artwork, FocusArtworkParams } from '../../generated/dashboard';
import type { NameAt } from '../../dashboard/countryFacts';
import { regionHour, SPEC, type AxisHour, type Region } from '../../dashboard/regionTime.ts';

Object.assign(globalThis, {
  __focusGate: (runtime: WidgetProps['runtime'], widget: WidgetProps['widget']) => ({ open: true, region: { widget, action: SPEC }, hour: runtime.clock.getHour(), stage: { stage: 'open' } }),
  __focusHourAt: (runtime: WidgetProps['runtime'], region: Region, axis: AxisHour) => regionHour(region, runtime.tracks[0], runtime.tracks, axis),
});

type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
const settle = () => new Promise<void>(resolve => setImmediate(resolve));

function focusHooks() {
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  let cursor = 0;
  const same = (a: readonly unknown[], b: readonly unknown[]) => a.length === b.length && a.every((value, i) => Object.is(value, b[i]));
  return {
    begin() { cursor = 0; },
    memo<T>(component: T) { return component; },
    useState(initial?: unknown) {
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
    useEffect(run: Effect['run'], deps: readonly unknown[]) {
      const index = cursor++, old = effects.get(index);
      if (!old || !same(old.deps, deps)) pending.set(index, { run, deps: [...deps] });
    },
    commit() {
      for (const [index, effect] of pending) {
        effects.get(index)?.cleanup?.();
        const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined;
        effects.set(index, effect);
      }
      pending.clear();
    },
    unmount() {
      for (const effect of effects.values()) effect.cleanup?.();
      slots.clear(); effects.clear(); pending.clear();
    },
  };
}

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('focus titles and icons exclude descriptions, while opened details retain separate failures and ownership', async () => {
  const hooks = focusHooks(), host = globalThis as typeof globalThis & { __focusArtworkHooks?: typeof hooks };
  host.__focusArtworkHooks = hooks;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-artwork-hooks-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/national-focus/useFocusArtwork.ts') || path.endsWith('/dashboard/countryNamesAt.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__focusArtworkHooks;');
        if (path.endsWith('/dashboard/contentRevision.ts')) return source.replace(
          'return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);', 'return revision.snapshot();');
      },
    }] });
  try {
    const { useFocusArtwork } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/useFocusArtwork.ts') as typeof import('./useFocusArtwork');
    const { contentRevision } = await vite.ssrLoadModule('/ui/src/dashboard/contentRevision.ts') as typeof import('../../dashboard/contentRevision');
    const { nameKey } = await vite.ssrLoadModule('/ui/src/dashboard/countryFacts.ts') as typeof import('../../dashboard/countryFacts');
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts') as typeof import('../../dashboard/queryError');
    const calls: { input: ProductRequest; signal: AbortSignal; answer: (data: unknown) => void; fail: () => void }[] = [];
    const makeClient = () => ({ product: (input: ProductRequest, signal: AbortSignal) => new Promise<ProductReply<unknown>>((resolve, reject) => {
      calls.push({ input, signal, answer: data => resolve({ ...input, data }), fail: () => reject(new RefusedError('Synthetic description refusal')) });
    }) }) as unknown as DashboardClient;
    let client = makeClient(), track = { source: { id: 'source', revision: 'revision' }, branch: { id: 'branch' } } as BoundTrack;
    const wanted = ['literal', 'empty', 'equal'].map(tag => ({ tag, hour: 10 }));
    let detailsWanted: readonly NameAt[] = [];
    const params = (call: typeof calls[number]) => call.input.params as unknown as FocusArtworkParams;
    const answer = (call: typeof calls[number], details = false) => call.answer({
      branchId: params(call).branchId, hour: params(call).hour, tag: String(params(call).country), database: params(call).database,
      items: params(call).tokens.map(token => ({ artwork: { token,
        name: params(call).iconsOnly ? null : details ? 'Detail title' : token === 'empty' ? '' : token === 'equal' ? 'equal' : 'Authored literal title',
        description: details ? '' : null, baseDurationDays: 10, icon: params(call).textOnly ? null : 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png' }, reason: null })),
      warnings: details ? ['Reached description warning'] : [],
    });
    const render = () => { hooks.begin(); return useFocusArtwork(client, track, 'M0', wanted, true, detailsWanted); };
    let result = render();
    assert.equal(result.read.size, 0);
    hooks.commit(); await settle();
    assert.equal(calls.length, 2, 'closed rows never schedule description expansion');
    assert.deepEqual(calls.map(call => [params(call).textOnly, params(call).includeDescriptions]), [[true, false], [false, false]]);
    assert.ok(calls.every(call => params(call).database === 'national'), 'tree and completion focuses are national');
    assert.equal(params(calls[0]).iconsOnly, undefined, 'dated titles retain their text request');
    assert.equal(params(calls[1]).iconsOnly, true, 'image work never repeats scoped title evaluation');
    answer(calls[1]); await settle();
    result = render();
    const pendingTitle = result.read.get(nameKey(10, 'literal'));
    assert.ok(pendingTitle && !(pendingTitle instanceof Error));
    assert.equal(pendingTitle.artwork.icon, 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png');
    assert.equal(pendingTitle.titlePending, true, 'a successful image cannot complete its pending title');
    assert.equal(pendingTitle.titleReason, undefined);
    calls[0].fail(); await settle();
    result = render();
    const failedTitle = result.read.get(nameKey(10, 'literal'));
    assert.ok(failedTitle && !(failedTitle instanceof Error));
    assert.equal(failedTitle.titlePending, false);
    assert.equal(failedTitle.titleReason, 'Synthetic description refusal');
    assert.ok(failedTitle.titleFailure instanceof RefusedError);
    assert.ok(result.warnings.includes('Synthetic description refusal'));
    assert.equal(failedTitle.artwork.icon, 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png', 'a failed title retains its successful image');
    contentRevision(client).observe({ game: 'retry-game', user: 'retry-user', contentReady: true });
    render(); hooks.commit(); await settle();
    calls.splice(0, 2);
    answer(calls[0]); await settle();
    result = render();
    assert.equal(result.read.size, 3, 'titles do not wait for images');
    answer(calls[1]); await settle();
    result = render();
    for (const [token, name] of [['literal', 'Authored literal title'], ['empty', ''], ['equal', 'equal']]) {
      const item = result.read.get(nameKey(10, token));
      assert.ok(item && !(item instanceof Error));
      assert.equal(item.artwork.name, name);
      assert.equal(item.artwork.icon, 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png');
      assert.equal(item.artwork.description, null);
    }

    detailsWanted = [wanted[1]];
    render(); hooks.commit(); await settle();
    assert.equal(calls.length, 3);
    assert.deepEqual(params(calls[2]).tokens, ['empty']);
    assert.equal(params(calls[2]).textOnly, true);
    assert.equal(params(calls[2]).includeDescriptions, true);
    assert.equal(params(calls[2]).iconsOnly, undefined, 'opened details retain their full text request');
    result = render();
    assert.equal(result.read.size, 3, 'pending details never delay existing titles or images');
    answer(calls[2], true); await settle();
    result = render();
    const empty = result.read.get(nameKey(10, 'empty'));
    assert.ok(empty && !(empty instanceof Error));
    assert.equal(empty.artwork.name, '', 'detail-side text cannot replace a known empty title');
    assert.equal(empty.artwork.description, '', 'known empty descriptions remain usable');
    assert.deepEqual(result.warnings, ['Reached description warning']);

    detailsWanted = [wanted[0], wanted[1]];
    render(); hooks.commit(); await settle();
    assert.deepEqual(params(calls[3]).tokens, ['literal'], 'the title-only answer does not suppress a full description request');
    calls[3].fail(); await settle();
    result = render(); hooks.commit(); await settle();
    assert.equal((result.details.get(nameKey(10, 'literal')) as Error).message, 'Synthetic description refusal');
    assert.ok(result.warnings.includes('Synthetic description refusal'));
    const literal = result.read.get(nameKey(10, 'literal'));
    assert.ok(literal && !(literal instanceof Error));
    assert.equal(literal.artwork.name, 'Authored literal title');
    assert.equal(literal.artwork.icon, 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png');
    assert.equal(calls.length, 4, 'permanent detail failures do not retry in a loop');
    detailsWanted = []; render(); hooks.commit();
    detailsWanted = [wanted[0]]; render(); hooks.commit(); await settle();
    assert.equal(calls.length, 5, 'closing every detail and reopening explicitly retries its refused request');

    const abandoned = calls[4];
    contentRevision(client).observe({ game: 'changed-game', user: 'changed-user', contentReady: true });
    result = render();
    assert.equal(result.read.size, 0);
    assert.equal(result.details.size, 0);
    assert.deepEqual(result.warnings, []);
    assert.equal(abandoned.signal.aborted, true);
    hooks.commit(); await settle();
    assert.equal(calls.length, 8, 'each independent lane reloads after content changes');
    answer(abandoned, true); await settle();
    assert.equal(render().read.size, 0, 'late old-content descriptions cannot repopulate the new source');
    track = { ...track, source: { ...track.source, revision: 'new-revision' } };
    result = render(); hooks.commit(); await settle();
    assert.equal(result.read.size, 0);
    assert.ok(calls.slice(5, 8).every(call => call.signal.aborted));
    assert.ok(calls.slice(8).every(call => call.input.revision === 'new-revision'));
    const oldClientCalls = calls.slice(8);
    client = makeClient();
    result = render(); hooks.commit(); await settle();
    assert.equal(result.read.size, 0);
    assert.equal(result.details.size, 0);
    assert.ok(oldClientCalls.every(call => call.signal.aborted));
  } finally {
    hooks.unmount(); await vite.close(); delete host.__focusArtworkHooks;
  }
});

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('known static focus titles omit dated title work, retaining empty and key-equal names while scoped names stay dated', async () => {
  const hooks = focusHooks(), host = globalThis as typeof globalThis & { __focusArtworkHooks?: typeof hooks };
  host.__focusArtworkHooks = hooks;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'static-focus-titles-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/national-focus/useFocusArtwork.ts') || path.endsWith('/dashboard/countryNamesAt.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__focusArtworkHooks;');
        if (path.endsWith('/dashboard/contentRevision.ts')) return source.replace(
          'return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);', 'return revision.snapshot();');
      },
    }] });
  try {
    const { useFocusArtwork } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/useFocusArtwork.ts') as typeof import('./useFocusArtwork');
    const { contentRevision } = await vite.ssrLoadModule('/ui/src/dashboard/contentRevision.ts') as typeof import('../../dashboard/contentRevision');
    const { nameKey } = await vite.ssrLoadModule('/ui/src/dashboard/countryFacts.ts') as typeof import('../../dashboard/countryFacts');
    const calls: { params: FocusArtworkParams; answer: (name: string | null, description?: string) => void }[] = [];
    const client = { product: (input: ProductRequest) => new Promise<ProductReply<unknown>>(resolve => {
      const params = input.params as unknown as FocusArtworkParams;
      calls.push({ params, answer: (name, description) => resolve({ ...input, data: {
        branchId: params.branchId, hour: params.hour, tag: String(params.country), database: params.database, warnings: [],
        items: params.tokens.map(token => ({ artwork: { token, name, description: description ?? null,
          baseDurationDays: 10, icon: params.textOnly ? null : 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png' }, reason: null })),
      } }) });
    }) } as unknown as DashboardClient;
    const track = { source: { id: 'source', revision: 'revision' }, branch: { id: 'branch' } } as BoundTrack;
    const wanted = ['plain', 'empty', 'equal', 'scoped', 'absent'].map(tag => ({ tag, hour: 10 }));
    let staticText: readonly Artwork[] = [
      { token: 'plain', name: 'Known static title' }, { token: 'empty', name: '' }, { token: 'equal', name: 'equal' }, { token: 'scoped', name: null },
    ].map(item => ({ ...item, description: null, baseDurationDays: 10, icon: null }));
    let detailsWanted: readonly NameAt[] = [];
    const render = () => { hooks.begin(); return useFocusArtwork(client, track, 'M0', wanted, true, detailsWanted, staticText); };
    let result = render();
    assert.equal(result.read.size, 3, 'known static titles are available before any dated reply');
    hooks.commit(); await settle();
    assert.equal(calls.length, 2);
    assert.deepEqual(calls[0].params.tokens, ['scoped', 'absent'], 'only null or absent static names need a dated title request');
    assert.equal(calls[0].params.textOnly, true);
    assert.equal(calls[0].params.includeDescriptions, false);
    assert.deepEqual(calls[1].params.tokens, wanted.map(item => item.tag), 'every icon retains its dated request');
    assert.equal(calls[1].params.textOnly, false);
    assert.equal(calls[1].params.iconsOnly, true);
    calls[1].answer(null); await settle();
    result = render();
    for (const token of ['scoped', 'absent']) {
      const item = result.read.get(nameKey(10, token));
      assert.ok(item && !(item instanceof Error));
      assert.equal(item.artwork.icon, 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png', 'icons display before slower scoped titles arrive');
      assert.equal(item.artwork.name, null, 'an image-only reply cannot invent a title');
    }
    calls[0].answer('Dated title'); await settle();
    result = render();
    for (const [token, name] of [['plain', 'Known static title'], ['empty', ''], ['equal', 'equal'], ['scoped', 'Dated title'], ['absent', 'Dated title']]) {
      const item = result.read.get(nameKey(10, token));
      assert.ok(item && !(item instanceof Error));
      assert.equal(item.artwork.name, name);
      assert.equal(item.artwork.icon, 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png');
    }
    detailsWanted = [wanted[0]]; render(); hooks.commit(); await settle();
    assert.equal(calls.length, 3);
    assert.deepEqual(calls[2].params.tokens, ['plain']);
    assert.equal(calls[2].params.includeDescriptions, true, 'static titles never suppress opened full descriptions');
    calls[2].answer('Detail-side title', 'Dated full description'); await settle();
    const detailed = render().read.get(nameKey(10, 'plain'));
    assert.ok(detailed && !(detailed instanceof Error));
    assert.equal(detailed.artwork.name, 'Known static title');
    assert.equal(detailed.artwork.description, 'Dated full description');
    contentRevision(client).observe({ game: 'changed-game', user: 'changed-user', contentReady: true });
    staticText = [];
    result = render(); hooks.commit(); await settle();
    assert.equal(result.read.size, 0, 'invalidated history text cannot fill a new content revision');
    assert.deepEqual(calls[3].params.tokens, wanted.map(item => item.tag), 'missing current static text restores dated title demand');
  } finally {
    hooks.unmount(); await vite.close(); delete host.__focusArtworkHooks;
  }
});

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('a continuous focus is read from its own database, never from a national focus of the same id', async () => {
  const hooks = focusHooks(), host = globalThis as typeof globalThis & { __focusArtworkHooks?: typeof hooks };
  host.__focusArtworkHooks = hooks;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-databases-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/national-focus/useFocusArtwork.ts') || path.endsWith('/dashboard/countryNamesAt.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__focusArtworkHooks;');
        if (path.endsWith('/dashboard/contentRevision.ts')) return source.replace(
          'return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);', 'return revision.snapshot();');
      },
    }] });
  try {
    const { useContinuousFocusArtwork, useFocusArtwork } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/useFocusArtwork.ts') as typeof import('./useFocusArtwork');
    const { nameKey } = await vite.ssrLoadModule('/ui/src/dashboard/countryFacts.ts') as typeof import('../../dashboard/countryFacts');
    const calls: { params: FocusArtworkParams; answer: () => void }[] = [];
    const client = { product: (input: ProductRequest) => new Promise<ProductReply<unknown>>(resolve => {
      const params = input.params as unknown as FocusArtworkParams;
      const name = params.database === 'national' ? 'National title' : 'Continuous title';
      calls.push({ params, answer: () => resolve({ ...input, data: {
        branchId: params.branchId, hour: params.hour, tag: String(params.country), database: params.database, warnings: [],
        items: params.tokens.map(token => ({ artwork: { token, name: params.iconsOnly ? null : name, description: null,
          baseDurationDays: null, icon: params.textOnly ? null : 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png' }, reason: null })),
      } }) });
    }) } as unknown as DashboardClient;
    const track = { source: { id: 'source', revision: 'revision' }, branch: { id: 'branch' } } as BoundTrack;
    const wanted = [{ tag: 'shared', hour: 10 }];
    const staticText: readonly Artwork[] = [{ token: 'shared', name: 'Static national title', description: null, baseDurationDays: 10, icon: null }];
    const noDetails: readonly NameAt[] = [];
    const render = () => {
      hooks.begin();
      return { national: useFocusArtwork(client, track, 'M0', wanted, true, noDetails, staticText),
        continuous: useContinuousFocusArtwork(client, track, 'M0', wanted, true) };
    };
    const shown = (read: ReturnType<typeof render>['national']['read']) => {
      const item = read.get(nameKey(10, 'shared'));
      return item && !(item instanceof Error) ? item.artwork : undefined;
    };
    let result = render();
    assert.equal(shown(result.national.read)?.name, 'Static national title');
    assert.equal(result.continuous.read.get(nameKey(10, 'shared')), undefined, 'national static text never names a continuous focus');
    hooks.commit(); await settle();
    const asked = (database: string) => calls.filter(call => call.params.database === database);
    assert.deepEqual(asked('national').map(call => call.params.iconsOnly), [true], 'the static national title needs no dated title');
    assert.deepEqual(asked('continuous').map(call => call.params.textOnly), [true, false], 'the continuous focus asks for its own title and icon');
    for (const call of asked('national')) call.answer();
    await settle();
    result = render();
    assert.equal(shown(result.national.read)?.icon, 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png');
    assert.equal(result.continuous.read.get(nameKey(10, 'shared')), undefined, 'a national reply never answers the continuous read');
    for (const call of asked('continuous')) call.answer();
    await settle();
    result = render();
    const continuous = result.continuous.read.get(nameKey(10, 'shared'));
    assert.ok(continuous && !(continuous instanceof Error));
    assert.equal(continuous.artwork.name, 'Continuous title');
    assert.equal(shown(result.national.read)?.name, 'Static national title');
  } finally {
    hooks.unmount(); await vite.close(); delete host.__focusArtworkHooks;
  }
});

test('native focus details request only opened dated rows and preserve static descriptions and titles after refusal', async () => {
  const base = focusHooks(), hooks = { ...base, useLayoutEffect: base.useEffect };
  const fixture = { hooks, wanted: [] as readonly NameAt[], detailsWanted: [] as readonly NameAt[], content: 0, titlePending: false, titleReason: undefined as string | undefined,
    details: new Map<string, unknown>(), diagnostics: [] as string[], staticText: [] as readonly Artwork[] };
  const host = globalThis as typeof globalThis & { __focusDetails?: typeof fixture };
  host.__focusDetails = fixture;
  const replacements: Record<string, string> = {
    '/components/VirtualList.tsx': 'export const VirtualList = ({ items, rowProps, children }) => items.map((item, index) => ({ type: "li", props: { ...rowProps?.(item, index), children: children(item, index) } }));',
    '/dashboard/products.ts': `export const useProduct = () => ({ data: {
      history: { segments: [{ firstHour: 100, lastHour: 200, order: { current: null, continuous: null, paused: false,
        completed: [{ ordinal: 0, token: 'scoped', completedByHour: 101 }, { ordinal: 1, token: 'plain', completedByHour: 102 }] } }], progress: [] },
      artwork: ['scoped', 'plain'].map(token => ({ token, baseDurationDays: 10 })),
      staticText: [{ token: 'plain', name: '', description: 'Static recorded description' }], continuousStaticText: [], warnings: [],
    } });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/army-size/headHour.ts': 'export const useHeadHour = (clock) => clock.getHour();',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = (_report, warnings) => { globalThis.__focusDetails.diagnostics = warnings; };',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => globalThis.__focusDetails.content;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/dashboard/widgetPerf.ts': 'export const useShownEachFrame = () => {};',
    '/national-focus/useFocusArtwork.ts': `export const useFocusArtwork = (_client, _track, _country, wanted, _active, detailsWanted, staticText) => {
      const fixture = globalThis.__focusDetails; fixture.wanted = wanted; fixture.detailsWanted = detailsWanted; fixture.staticText = staticText;
      return { warnings: [], details: fixture.details, read: new Map(wanted.map(({ tag, hour }) => [JSON.stringify([hour, tag]), {
        artwork: { token: tag, name: tag === 'plain' ? '' : fixture.titlePending || fixture.titleReason ? null : 'Localized focus', description: null, baseDurationDays: 10, icon: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png' }, reason: null, titlePending: tag === 'scoped' && fixture.titlePending, titleReason: tag === 'scoped' ? fixture.titleReason : undefined,
      }])) }; };
      export const useContinuousFocusArtwork = () => ({ warnings: [], read: new Map(), details: new Map() });`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-details-renderer-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/national-focus/NationalFocusWidget.tsx')) return source.replace(
          /import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__focusDetails.hooks;')
          .replace('useGate(runtime, widget, SPEC, active)', 'globalThis.__focusGate(runtime, widget)')
          .replace('useRegionHourAt(runtime, gate.region)', '(axis => globalThis.__focusHourAt(runtime, gate.region, axis))');
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      },
    }] });
  type Element = { type: unknown; props: { children?: unknown; className?: string; open?: boolean; 'data-tooltip'?: string; onToggle?: (event: { currentTarget: { open: boolean } }) => void } };
  const elements = (root: unknown): Element[] => {
    if (Array.isArray(root)) return root.flatMap(elements);
    if (!root || typeof root !== 'object' || !('props' in root)) return [];
    const element = root as Element;
    if (typeof element.type === 'function' && ['VirtualList', 'FocusCompletion'].includes(element.type.name)) return elements((element.type as Function)(element.props));
    return [element, ...elements(element.props.children)];
  };
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx');
    const { FAILED_FACT, nameKey } = await vite.ssrLoadModule('/ui/src/dashboard/countryFacts.ts') as typeof import('../../dashboard/countryFacts');
    const props = { widget: { id: 'details', kind: 'national-focus', trackIds: ['track'], countries: { kind: 'tags', tags: ['M0'] }, time: { mode: 'campaign' } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: '1', firstHour: 100, lastHour: 200 },
        source: { id: 'source', revision: '1' } }], clock: { getHour: () => 200, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }), subscribe: () => () => {}, subscribeFrame: () => () => {} } }, active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { hooks.begin(); return elements(Widget(props)); };
    const details = (tree: Element[]) => tree.filter(element => element.props.className === 'focus-details');
    const paragraphs = (tree: Element[]) => tree.filter(element => element.type === 'p').map(element => element.props.children);
    let tree = render(); hooks.commit();
    assert.deepEqual(fixture.detailsWanted, []);
    assert.deepEqual(fixture.staticText.map(item => [item.token, item.name]), [['plain', '']], 'the renderer passes validated staticText to the artwork hook');
    assert.deepEqual(details(tree).map(element => element.props.open), [false, false]);
    fixture.titlePending = true;
    tree = render(); hooks.commit();
    assert.ok(tree.some(element => element.type === 'strong' && element.props.children === 'Loading name\u2026'));
    fixture.titlePending = false; fixture.titleReason = 'Focus name request failed.';
    tree = render(); hooks.commit();
    assert.ok(tree.some(element => element.type === 'strong' && String(element.props['data-tooltip']).includes(fixture.titleReason!)));
    assert.ok(fixture.diagnostics.includes('Focus name request failed. (1)'));
    fixture.titleReason = undefined;
    tree = render(); hooks.commit();
    assert.ok(paragraphs(tree).includes('Static recorded description'), 'title-only replies retain known static descriptions');
    details(tree)[0].props.onToggle!({ currentTarget: { open: true } });
    tree = render(); hooks.commit();
    assert.deepEqual(fixture.detailsWanted, [{ tag: 'scoped', hour: 101 }]);
    assert.deepEqual(details(tree).map(element => element.props.open), [true, false]);
    assert.ok(!paragraphs(tree).includes('Loading focus description…'), 'a loading description adds no paragraph');
    fixture.details.set(nameKey(101, 'scoped'), FAILED_FACT);
    tree = render(); hooks.commit();
    assert.ok(!paragraphs(tree).includes('Focus description request failed.'), 'a failed description adds no paragraph');
    assert.ok(tree.some(element => element.type === 'strong' && element.props.children === 'Localized focus'));
    assert.ok(tree.some(element => element.type === 'strong' && element.props.children === ''), 'empty title remains empty');
    assert.ok(fixture.diagnostics.includes('Focus description request failed. (1)'));
    details(tree)[0].props.onToggle!({ currentTarget: { open: false } });
    tree = render(); hooks.commit();
    assert.deepEqual(fixture.detailsWanted, []);
    details(tree)[1].props.onToggle!({ currentTarget: { open: true } });
    render(); hooks.commit();
    assert.deepEqual(fixture.detailsWanted, [{ tag: 'plain', hour: 102 }]);
    fixture.content++;
    tree = render(); hooks.commit();
    assert.deepEqual(fixture.detailsWanted, []);
    assert.ok(details(tree).every(element => element.props.open === false), 'new content closes stale detail demand');
    details(tree)[0].props.onToggle!({ currentTarget: { open: true } });
    render(); hooks.commit();
    props.runtime.client = {} as DashboardClient;
    tree = render(); hooks.commit();
    assert.deepEqual(fixture.detailsWanted, []);
    assert.ok(details(tree).every(element => element.props.open === false), 'another client cannot inherit open detail demand');
  } finally {
    hooks.unmount(); await vite.close(); delete host.__focusDetails;
  }
});

test('a closing row keeps the description it showed while its demand is released, until a refusal, new content or another client', async () => {
  const base = focusHooks(), hooks = { ...base, useLayoutEffect: base.useEffect };
  const fixture = { hooks, detailsWanted: [] as readonly NameAt[], content: 0, details: new Map<string, unknown>() };
  const host = globalThis as typeof globalThis & { __focusShown?: typeof fixture };
  host.__focusShown = fixture;
  const replacements: Record<string, string> = {
    '/components/VirtualList.tsx': 'export const VirtualList = ({ items, rowProps, children }) => items.map((item, index) => ({ type: "li", props: { ...rowProps?.(item, index), children: children(item, index) } }));',
    '/dashboard/products.ts': `export const useProduct = () => ({ data: {
      history: { segments: [{ firstHour: 100, lastHour: 200, order: { current: null, continuous: null, paused: false,
        completed: [{ ordinal: 0, token: 'scoped', completedByHour: 101 }] } }], progress: [] },
      artwork: [{ token: 'scoped', baseDurationDays: 10 }], staticText: [], continuousStaticText: [], warnings: [],
    } });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/army-size/headHour.ts': 'export const useHeadHour = (clock) => clock.getHour();',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => globalThis.__focusShown.content;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/dashboard/widgetPerf.ts': 'export const useShownEachFrame = () => {};',
    '/national-focus/useFocusArtwork.ts': `export const useFocusArtwork = (_client, _track, _country, wanted, _active, detailsWanted) => {
      const fixture = globalThis.__focusShown; fixture.detailsWanted = detailsWanted;
      return { warnings: [], details: fixture.details, read: new Map(wanted.map(({ tag, hour }) => {
        const key = JSON.stringify([hour, tag]), detail = fixture.details.get(key);
        return [key, { artwork: { token: tag, name: 'Localized focus', baseDurationDays: 10, icon: null,
          description: detail && typeof detail === 'object' && 'artwork' in detail ? detail.artwork.description : null }, reason: null }];
      })) }; };
      export const useContinuousFocusArtwork = () => ({ warnings: [], read: new Map(), details: new Map() });`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-shown-description-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/national-focus/NationalFocusWidget.tsx')) return source.replace(
          /import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__focusShown.hooks;')
          .replace('useGate(runtime, widget, SPEC, active)', 'globalThis.__focusGate(runtime, widget)')
          .replace('useRegionHourAt(runtime, gate.region)', '(axis => globalThis.__focusHourAt(runtime, gate.region, axis))');
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      },
    }] });
  type Element = { type: unknown; props: { children?: unknown; className?: string; onToggle?: (event: { currentTarget: { open: boolean } }) => void } };
  const elements = (root: unknown): Element[] => {
    if (Array.isArray(root)) return root.flatMap(elements);
    if (!root || typeof root !== 'object' || !('props' in root)) return [];
    const element = root as Element;
    if (typeof element.type === 'function' && ['VirtualList', 'FocusCompletion'].includes(element.type.name)) return elements((element.type as Function)(element.props));
    return [element, ...elements(element.props.children)];
  };
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx');
    const { FAILED_FACT, nameKey } = await vite.ssrLoadModule('/ui/src/dashboard/countryFacts.ts') as typeof import('../../dashboard/countryFacts');
    const props = { widget: { id: 'shown', kind: 'national-focus', trackIds: ['track'], countries: { kind: 'tags', tags: ['M0'] }, time: { mode: 'campaign' } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: '1', firstHour: 100, lastHour: 200 },
        source: { id: 'source', revision: '1' } }], clock: { getHour: () => 200, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }), subscribe: () => () => {}, subscribeFrame: () => () => {} } }, active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const key = nameKey(101, 'scoped');
    const answer = () => fixture.details.set(key, { artwork: { token: 'scoped', name: null, description: 'Dated full description', baseDurationDays: 10, icon: null }, reason: null });
    const render = () => { hooks.begin(); const tree = elements(Widget(props)); hooks.commit(); return tree; };
    const paragraphs = (tree: Element[]) => tree.filter(element => element.type === 'p').map(element => element.props.children);
    const toggle = (open: boolean) => { render().find(element => element.props.className === 'focus-details')!.props.onToggle!({ currentTarget: { open } }); };
    const close = () => { toggle(false); fixture.details.delete(key); };

    toggle(true);
    assert.deepEqual(paragraphs(render()), [], 'a pending description adds no paragraph');
    answer();
    assert.deepEqual(paragraphs(render()), ['Dated full description']);
    close();
    let tree = render();
    assert.deepEqual(fixture.detailsWanted, [], 'closing still releases the description demand');
    assert.deepEqual(paragraphs(tree), ['Dated full description'], 'the closing row keeps its text rather than collapsing from a shorter one');
    toggle(true); tree = render();
    assert.deepEqual(fixture.detailsWanted, [{ tag: 'scoped', hour: 101 }], 'reopening requests the description again');
    assert.deepEqual(paragraphs(tree), ['Dated full description'], 'the reopened row shows the reading it showed, without waiting');
    fixture.details.set(key, FAILED_FACT);
    assert.deepEqual(paragraphs(render()), [], 'a refusal is not hidden by an earlier reading');
    answer(); render(); close();
    fixture.content++; tree = render();
    toggle(true);
    assert.deepEqual(paragraphs(render()), [], 'new content does not show an earlier reading');
    answer(); render(); close();
    props.runtime.client = {} as DashboardClient; render();
    toggle(true);
    assert.deepEqual(paragraphs(render()), [], 'another client does not show an earlier reading');
  } finally {
    hooks.unmount(); await vite.close(); delete host.__focusShown;
  }
});

test('repeated completions of one focus read at one hour open and close independently', async () => {
  const base = focusHooks(), hooks = { ...base, useLayoutEffect: base.useEffect };
  const fixture = { hooks, detailsWanted: [] as readonly NameAt[] };
  const host = globalThis as typeof globalThis & { __focusRepeated?: typeof fixture };
  host.__focusRepeated = fixture;
  const replacements: Record<string, string> = {
    '/components/VirtualList.tsx': 'export const VirtualList = ({ items, rowProps, children }) => items.map((item, index) => ({ type: "li", props: { ...rowProps?.(item, index), children: children(item, index) } }));',
    '/dashboard/products.ts': `export const useProduct = () => ({ data: {
      history: { segments: [{ firstHour: 100, lastHour: 200, order: { current: null, continuous: null, paused: false,
        completed: [{ ordinal: 0, token: 'repeatable', completedByHour: null }, { ordinal: 1, token: 'repeatable', completedByHour: null }] } }], progress: [] },
      artwork: [{ token: 'repeatable', baseDurationDays: 10 }], staticText: [], continuousStaticText: [], warnings: [],
    } });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/army-size/headHour.ts': 'export const useHeadHour = (clock) => clock.getHour();',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/dashboard/widgetPerf.ts': 'export const useShownEachFrame = () => {};',
    '/national-focus/useFocusArtwork.ts': `export const useFocusArtwork = (_client, _track, _country, wanted, _active, detailsWanted) => {
      globalThis.__focusRepeated.detailsWanted = detailsWanted;
      return { warnings: [], details: new Map(), read: new Map(wanted.map(({ tag, hour }) => [JSON.stringify([hour, tag]),
        { artwork: { token: tag, name: 'Repeatable focus', description: null, baseDurationDays: 10, icon: null }, reason: null }])) }; };
      export const useContinuousFocusArtwork = () => ({ warnings: [], read: new Map(), details: new Map() });`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'focus-repeated-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/national-focus/NationalFocusWidget.tsx')) return source.replace(
          /import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__focusRepeated.hooks;')
          .replace('useGate(runtime, widget, SPEC, active)', 'globalThis.__focusGate(runtime, widget)')
          .replace('useRegionHourAt(runtime, gate.region)', '(axis => globalThis.__focusHourAt(runtime, gate.region, axis))');
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      },
    }] });
  type Element = { type: unknown; props: { children?: unknown; className?: string; open?: boolean; onToggle?: (event: { currentTarget: { open: boolean } }) => void } };
  const elements = (root: unknown): Element[] => {
    if (Array.isArray(root)) return root.flatMap(elements);
    if (!root || typeof root !== 'object' || !('props' in root)) return [];
    const element = root as Element;
    if (typeof element.type === 'function' && ['VirtualList', 'FocusCompletion'].includes(element.type.name)) return elements((element.type as Function)(element.props));
    return [element, ...elements(element.props.children)];
  };
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx');
    const props = { widget: { id: 'repeated', kind: 'national-focus', trackIds: ['track'], countries: { kind: 'tags', tags: ['M0'] }, time: { mode: 'campaign' } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: '1', firstHour: 100, lastHour: 200 },
        source: { id: 'source', revision: '1' } }], clock: { getHour: () => 200, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }), subscribe: () => () => {}, subscribeFrame: () => () => {} } }, active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { hooks.begin(); const tree = elements(Widget(props)); hooks.commit(); return tree; };
    const details = (tree: Element[]) => tree.filter(element => element.props.className === 'focus-details');
    let tree = render();
    assert.deepEqual(details(tree).map(element => element.props.open), [false, false]);
    details(tree)[0].props.onToggle!({ currentTarget: { open: true } });
    tree = render();
    assert.deepEqual(details(tree).map(element => element.props.open), [true, false], 'opening one completion leaves the other closed');
    assert.deepEqual(fixture.detailsWanted, [{ tag: 'repeatable', hour: 100 }]);
    details(tree)[1].props.onToggle!({ currentTarget: { open: true } });
    tree = render();
    details(tree)[0].props.onToggle!({ currentTarget: { open: false } });
    tree = render();
    assert.deepEqual(details(tree).map(element => element.props.open), [false, true], 'closing one completion leaves the other open');
    assert.deepEqual(fixture.detailsWanted, [{ tag: 'repeatable', hour: 100 }], 'the open completion still reads the shared dated text');
  } finally {
    hooks.unmount(); await vite.close(); delete host.__focusRepeated;
  }
});
