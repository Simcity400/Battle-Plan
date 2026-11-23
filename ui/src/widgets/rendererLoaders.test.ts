import assert from 'node:assert/strict';
import test from 'node:test';
import { existsSync, readFileSync } from 'node:fs';
import { widgetMetas } from './definitions.ts';
import { loadWidgetRenderer, rendererLoaders } from './rendererLoaders.ts';
import { createServer } from 'vite';
test('all installed kinds have independent lazy loaders and unknown saved kinds remain unknown', async () => {
  assert.deepEqual(Object.keys(rendererLoaders).sort(), widgetMetas.map(meta => meta.kind).sort());
  assert.equal(await loadWidgetRenderer('uninstalled-synthetic-kind'), undefined);
  const registry = readFileSync(new URL('./index.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(registry, /import \w+ from ['"]\.\/[^'"]+['"]/);
  assert.match(registry, /'map': lazyWidget\('map', \['render'\]\)/);
});

test('every installed widget registers its body and all declared lazy slots', () => {
  const registry = readFileSync(new URL('./index.ts', import.meta.url), 'utf8');
  for (const meta of widgetMetas) {
    const slots = registry.match(new RegExp(`'${meta.kind}': lazyWidget\\('${meta.kind}', \\[([^\\]]*)\\]\\)`))?.[1];
    assert.ok(slots?.includes("'render'"), `${meta.kind} must mount its body`);
    const entry = ['index.ts', 'index.tsx'].map(name => new URL(`./${meta.kind}/${name}`, import.meta.url)).find(existsSync);
    assert.ok(entry, `${meta.kind} must provide an index.ts or index.tsx entry`);
    const source = readFileSync(entry, 'utf8');
    for (const slot of ['countryChip']) {
      if (new RegExp(`\\b${slot}\\s*[:,]`).test(source)) assert.ok(slots?.includes(`'${slot}'`), `${meta.kind} must retain ${slot}`);
    }
  }
});

test('every widget module loads with its registered lazy body and the catalogue keeps its preset hook', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    await vite.ssrLoadModule('/ui/src/widgets/index.ts');
    const { loadWidgetRenderer: load } = await vite.ssrLoadModule('/ui/src/widgets/rendererLoaders.ts');
    const { widgetDefinition } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    for (const meta of widgetMetas) {
      const registered = widgetDefinition(meta.kind);
      const renderer = await load(meta.kind);
      assert.equal(typeof renderer.render, 'function', `${meta.kind} body loads`);
      assert.equal(typeof registered.render, 'function', `${meta.kind} body is registered`);
      for (const slot of ['countryChip']) {
        if (renderer[slot]) assert.equal(typeof registered[slot], 'function', `${meta.kind} ${slot} is registered`);
      }
      if (renderer.usePresets) assert.equal(registered.usePresets, renderer.usePresets, `${meta.kind} presets are available on first catalogue open`);
    }
  } finally { await vite.close(); }
});

test('prototype-like unknown widget kinds have no renderer', async () => {
  for (const kind of ['constructor', '__proto__', 'toString', 'hasOwnProperty']) assert.equal(await loadWidgetRenderer(kind), undefined);
});
