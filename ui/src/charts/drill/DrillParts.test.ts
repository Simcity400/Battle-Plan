import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { orderedDrillParts } from './parts.ts';

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{
    name: 'drill-row-hooks', enforce: 'pre',
    transform(code, id) {
      if (id.replaceAll('\\', '/').endsWith('/drill/DrillParts.tsx')) return code.replace(
        "import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';",
        'const useMemo = fn => fn(); const useCallback = fn => fn; const useRef = current => ({ current }); const useState = initial => [initial, () => {}]; const useEffect = effect => globalThis.__drillPartEffects?.push(effect); const useLayoutEffect = () => {};');
    },
  }] });
const { default: DrillParts } = await vite.ssrLoadModule('/ui/src/charts/drill/DrillParts.tsx').finally(() => vite.close()) as typeof import('./DrillParts.tsx');


test('effects replayed after their cleanup (React StrictMode) report the shown rows again', () => {
  const shared = globalThis as typeof globalThis & { __drillPartEffects?: (() => unknown)[] };
  const calls: (readonly string[])[] = [];
  try {
    shared.__drillPartEffects = [];
    DrillParts({ parts: [{ id: 'native part', label: 'Part', value: 1 }], format: String, onVisible: ids => calls.push(ids) });
    const effects = shared.__drillPartEffects;
    for (const cleanup of effects.map(effect => effect())) if (typeof cleanup === 'function') cleanup();
    effects.forEach(effect => effect());
    assert.deepEqual(calls, [['native part'], [], ['native part']], 'rows shown after the replay must have their labels asked for');
  } finally { delete shared.__drillPartEffects; }
});


test('raw rows sort exactly with stable native IDs and render exact shares independently of plotting proxies', () => {
  const rows = orderedDrillParts([{ id: 'b', label: 'A name arriving later', value: 1, raw: 1n },
    { id: 'a', label: 'Z opaque token', value: 1, raw: 1n },
    { id: 'wide', label: 'Wide', value: 9007199254740992, raw: 9007199254740993n },
    { id: 'less', label: 'Less', value: 9007199254740992, raw: 9007199254740992n }]);
  assert.deepEqual(rows.map(row => row.id), ['wide', 'less', 'a', 'b']);
  const tree = DrillParts({ parts: [], format: String, total: 1,
    unattributed: { label: 'Other', value: 9007199254740992, raw: 9007199254740993n,
      text: '9,007,199,254,740,993', shareText: '900719925474099300%' } });
  assert.match(renderToStaticMarkup(tree), /900719925474099300%/);
  assert.match(renderToStaticMarkup(tree), /9,007,199,254,740,993/);
});

