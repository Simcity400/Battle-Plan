import assert from 'node:assert/strict';
import test from 'node:test';
import { existsSync, readFileSync } from 'node:fs';
import { createServer } from 'vite';
import { widgetMetas } from './definitions.ts';
import { FAMILY_VIEW_UI, familyViewsForKind, manifestSlotsForKind } from './manifest.ts';
import { buildRendererLoaders, isPlaybackActive, loadWidgetRenderer, preloadAllWidgetRenderers, rendererKinds, setPlaybackActive } from './rendererLoaders.ts';
import { viewKey, type FamilyViewEntry } from '../families/manifest.ts';

test('all installed kinds have independent lazy loaders and unknown saved kinds remain unknown', async () => {
  assert.deepEqual([...rendererKinds()].sort(), widgetMetas.map(meta => meta.kind).sort());
  assert.equal(await loadWidgetRenderer('uninstalled-synthetic-kind'), undefined);
  const registry = readFileSync(new URL('./index.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(registry, /import \w+ from ['"]\.\/[^'"]+['"]/);
  assert.deepEqual(manifestSlotsForKind('map'), ['render']);
});

test('every installed widget registers its body and all declared lazy slots', () => {
  for (const meta of widgetMetas) {
    const slots = manifestSlotsForKind(meta.kind);
    assert.ok(slots.includes('render'), `${meta.kind} must mount its body`);
    const entry = ['index.ts', 'index.tsx', 'family.tsx'].map(name => new URL(`./${meta.kind}/${name}`, import.meta.url)).find(existsSync);
    if (!entry && familyViewsForKind(meta.kind).length && !Object.hasOwn(FAMILY_VIEW_UI, meta.kind)) continue;
    assert.ok(entry, `${meta.kind} must provide an index.ts, index.tsx or family.tsx entry`);
    const source = readFileSync(entry, 'utf8');
    for (const slot of ['countryChip'] as const) {
      if (new RegExp(`\\b${slot}\\s*[:,]`).test(source)) assert.ok(slots.includes(slot), `${meta.kind} must retain ${slot}`);
    }
  }
});

test('every widget module loads with its registered lazy body and the catalogue keeps its preset hook', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    await vite.ssrLoadModule('/ui/src/widgets/index.ts');
    const { loadWidgetRenderer: load, setPlaybackActive: setPlaying } = await vite.ssrLoadModule('/ui/src/widgets/rendererLoaders.ts');
    const { widgetDefinition } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    const { familyHost } = await vite.ssrLoadModule('/ui/src/families/host/registry.tsx');
    const clock = {};
    setPlaying(clock, true);
    try {
      const first = await load(widgetMetas[0]!.kind);
      assert.equal(typeof first.render, 'function', 'on-demand loadWidgetRenderer still loads while playback is active');
    } finally {
      setPlaying(clock, false);
    }
    for (const meta of widgetMetas) {
      const registered = widgetDefinition(meta.kind);
      const renderer = await load(meta.kind);
      assert.ok(
        typeof renderer.render === 'function' || (renderer.declaration !== undefined && familyHost(renderer.declaration.family) !== undefined),
        `${meta.kind} body or family declaration loads`,
      );
      if (renderer.declaration) {
        assert.equal(renderer.declaration.kind, meta.kind, `${meta.kind} declaration matches kind`);
      }
      assert.equal(typeof registered.render, 'function', `${meta.kind} body is registered`);
      for (const slot of ['countryChip']) {
        if (renderer[slot]) assert.equal(typeof registered[slot], 'function', `${meta.kind} ${slot} is registered`);
      }
      if (renderer.usePresets) assert.equal(registered.usePresets, renderer.usePresets, `${meta.kind} presets are available on first catalogue open`);
    }
    const { FAMILY_VIEWS: views, LEGACY_WIDGETS: legacy } = await vite.ssrLoadModule('/ui/src/widgets/manifest.ts');
    const legacyKeys = new Set((legacy as readonly { kind: string; view?: string }[]).map(item => viewKey(item.kind, item.view)));
    for (const entry of views as readonly FamilyViewEntry[]) {
      const key = viewKey(entry.kind, entry.view);
      assert.equal(legacyKeys.has(key), false, `${key} is not shadowed by a LEGACY_WIDGETS entry`);
      const renderer = await load(entry.kind, entry.view);
      const spec = await entry.loadSpec();
      const drawn = renderer.declaration;
      const named = `${key} draws the declaration its manifest entry names`;
      assert.deepEqual([drawn?.family, drawn?.kind], [spec.declaration.family, spec.declaration.kind], named);
      assert.equal(drawn?.projection, spec.declaration.projection, named);
    }
    const { LEGACY_WIDGETS: legacy } = await vite.ssrLoadModule('/ui/src/widgets/legacyManifest.ts');
    const { OPTION_CHOICES: kindChoices } = await vite.ssrLoadModule('/ui/src/widgets/manifest.ts');
    const choiceHooks = new Map<string, Set<unknown>>();
    for (const entry of [...views as readonly FamilyViewEntry[], ...legacy as readonly { kind: string; view?: string }[]]) {
      const hooks = choiceHooks.get(entry.kind) ?? new Set<unknown>();
      const kindHook = Object.hasOwn(kindChoices, entry.kind) ? await kindChoices[entry.kind]() : undefined;
      hooks.add(kindHook ?? (await load(entry.kind, entry.view)).useOptionChoices);
      choiceHooks.set(entry.kind, hooks);
    }
    const lost = [...choiceHooks].filter(([, hooks]) => hooks.size > 1).map(([kind]) => kind);
    assert.deepEqual(lost, [], 'every view of a kind loads the recording’s option choices, whichever view loads first');
  } finally { await vite.close(); }
});

test('prototype-like unknown widget kinds have no renderer', async () => {
  for (const kind of ['constructor', '__proto__', 'toString', 'hasOwnProperty']) assert.equal(await loadWidgetRenderer(kind), undefined);
});

test('idle preloading never runs while playback is active, respects idle budget, and serializes modulepreload fetches', async () => {
  const clock = {};
  const queued: ((deadline: { didTimeout: boolean; timeRemaining(): number }) => void)[] = [];
  const preloaded: string[] = [];
  const loaded: string[] = [];
  const resolvers: (() => void)[] = [];
  const stop = preloadAllWidgetRenderers({
    kinds: ['a', 'b', 'c'],
    chunks: { a: './assets/widget-a.js', b: './assets/widget-b.js', c: './assets/widget-c.js' },
    schedule: run => queued.push(run),
    cancel: () => {},
    preload: url => new Promise<void>(resolve => { preloaded.push(url); resolvers.push(resolve); }),
    load: async kind => { loaded.push(kind); },
  });
  try {
    assert.equal(queued.length, 1);
    setPlaybackActive(clock, true);
    assert.equal(isPlaybackActive(), true);
    queued.shift()!({ didTimeout: false, timeRemaining: () => 40 });
    assert.deepEqual([preloaded, loaded], [[], []], 'nothing preloads while playing');
    assert.equal(queued.length, 0, 'no idle timer spins while playing');
    setPlaybackActive(clock, false);
    assert.equal(isPlaybackActive(), false);
    assert.equal(queued.length, 1, 'preloading resumes once playback pauses');
    queued.shift()!({ didTimeout: true, timeRemaining: () => 0 });
    assert.deepEqual(preloaded, [], 'a timed-out deadline is skipped');
    queued.shift()!({ didTimeout: false, timeRemaining: () => 4 });
    assert.deepEqual(preloaded, [], 'an insufficient idle slice is skipped');
    queued.shift()!({ didTimeout: false, timeRemaining: () => 30 });
    assert.deepEqual(preloaded, ['./assets/widget-a.js'], 'one chunk starts preloading');
    assert.equal(queued.length, 0, 'next chunk waits until modulepreload settles before scheduling');
    setPlaybackActive(clock, true);
    resolvers.shift()!();
    await new Promise<void>(resolve => setImmediate(resolve));
    assert.equal(queued.length, 0, 'if playback starts while a modulepreload is in flight, no further chunk schedules');
    setPlaybackActive(clock, false);
    assert.equal(queued.length, 1);
    queued.shift()!({ didTimeout: false, timeRemaining: () => 30 });
    assert.deepEqual(preloaded, ['./assets/widget-a.js', './assets/widget-b.js']);
    assert.equal(queued.length, 0);
    resolvers.shift()!();
    await new Promise<void>(resolve => setImmediate(resolve));
    queued.shift()!({ didTimeout: false, timeRemaining: () => 30 });
    resolvers.shift()!();
    await new Promise<void>(resolve => setImmediate(resolve));
    assert.deepEqual(preloaded, ['./assets/widget-a.js', './assets/widget-b.js', './assets/widget-c.js']);
    assert.deepEqual(loaded, [], 'modulepreload avoids main-thread module evaluation');
    assert.equal(queued.length, 0);
  } finally {
    setPlaybackActive(clock, false);
    stop();
  }
});

test('two family views of one kind get their own loader, each loading its own declaration', async () => {
  const declarationA = { name: 'A' };
  const declarationB = { name: 'B' };
  const spec = (view: string, declaration: object) => async () => ({ family: 'bars', kind: 'synthetic-two-views', view, projection: {} as never, declaration });
  const loaders = buildRendererLoaders([], [
    { kind: 'synthetic-two-views', view: 'shares', family: 'bars', loadSpec: spec('shares', declarationA) },
    { kind: 'synthetic-two-views', view: 'race', family: 'bars', loadSpec: spec('race', declarationB) },
  ], {});
  const shares = loaders.get(viewKey('synthetic-two-views', 'shares'));
  const race = loaders.get(viewKey('synthetic-two-views', 'race'));
  assert.ok(shares && race && shares !== race);
  assert.equal((await shares.load()).default.declaration, declarationA);
  assert.equal((await race.load()).default.declaration, declarationB);
});
