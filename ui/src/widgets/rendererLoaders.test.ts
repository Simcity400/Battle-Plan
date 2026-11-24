import assert from 'node:assert/strict';
import test from 'node:test';
import { existsSync, readFileSync } from 'node:fs';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { widgetMetas } from './definitions.ts';
import { manifestSlotsForKind } from './manifest.ts';
import { isPlaybackActive, loadWidgetRenderer, preloadAllWidgetRenderers, rendererLoaders, setPlaybackActive } from './rendererLoaders.ts';

test('all installed kinds have independent lazy loaders and unknown saved kinds remain unknown', async () => {
  assert.deepEqual(Object.keys(rendererLoaders).sort(), widgetMetas.map(meta => meta.kind).sort());
  assert.equal(await loadWidgetRenderer('uninstalled-synthetic-kind'), undefined);
  const registry = readFileSync(new URL('./index.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(registry, /import \w+ from ['"]\.\/[^'"]+['"]/);
  assert.deepEqual(manifestSlotsForKind('map'), ['render']);
});

test('every installed widget registers its body and all declared lazy slots', () => {
  for (const meta of widgetMetas) {
    const slots = manifestSlotsForKind(meta.kind);
    assert.ok(slots.includes('render'), `${meta.kind} must mount its body`);
    const entry = ['index.ts', 'index.tsx'].map(name => new URL(`./${meta.kind}/${name}`, import.meta.url)).find(existsSync);
    assert.ok(entry, `${meta.kind} must provide an index.ts or index.tsx entry`);
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
        typeof renderer.render === 'function' || renderer.declaration !== undefined,
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

  } finally { await vite.close(); }
});

test('a kind whose views are on two families renders the family of its active view', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'family-view-ssr', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/dashboard/follow/useFollowed.ts')) {
          return source.replace(/export function usePipeline\(/, 'function real_usePipeline(') + '\nexport function usePipeline() { return null; }';
        }
      },
    }] });
  try {
    await vite.ssrLoadModule('/ui/src/widgets/index.ts');
    const { loadWidgetRenderer: load } = await vite.ssrLoadModule('/ui/src/widgets/rendererLoaders.ts');
    const { loadFamilyViewSpec } = await vite.ssrLoadModule('/ui/src/widgets/manifest.ts');
    const { widgetDefinition } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    await load('decryption');
    await loadFamilyViewSpec('decryption', 'ciphers');
    await loadFamilyViewSpec('decryption', 'matrix');
    const Body = widgetDefinition('decryption').render;
    const runtime = { tracks: [], campaignTracks: [], defaultRoleId: null, maps: {}, client: { available: true },
      clock: { getHour: () => 0, getSnapshot: () => ({ playing: false, hoursPerSecond: 1 }), subscribe: () => () => {}, subscribeFrame: () => () => {} } };
    const body = (view: string) => renderToStaticMarkup(createElement(Body, {
      widget: { id: 'd', kind: 'decryption', configVersion: 1, title: 'Decryption', campaign: 'default', trackIds: [], time: { mode: 'follow' }, countries: { kind: 'all' }, options: { view } },
      runtime, active: true, mapOptions: { fog: true, country: null }, headerSlot: null, onDiagnostics() {},
    }));
    assert.match(body('ciphers'), /decryption races/, 'the Ciphers view renders its meters view');
    assert.match(body('matrix'), /who reads whom/, 'the Who reads whom view renders its heat-matrix view');
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
