import assert from 'node:assert/strict';
import test from 'node:test';
import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
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
function nodes(node: ReactNode): ReactElement<Record<string, unknown>>[] {
  if (Array.isArray(node)) return node.flatMap(nodes);
  if (!isValidElement<Record<string, unknown>>(node)) return [];
  return [node, ...nodes(node.props.children as ReactNode)];
}

test('the first visible report is safe without a callback and releases a supplied callback on unmount', () => {
  const shared = globalThis as typeof globalThis & { __drillPartEffects?: (() => unknown)[] };
  const calls: (readonly string[])[] = [];
  try {
    for (const onVisible of [undefined, (ids: readonly string[]) => calls.push(ids)]) {
      shared.__drillPartEffects = [];
      DrillParts({ parts: [{ id: 'native part', label: 'Part', value: 1 }], format: String, onVisible });
      const cleanups = shared.__drillPartEffects.map(effect => effect());
      for (const cleanup of cleanups) if (typeof cleanup === 'function') cleanup();
    }
    assert.deepEqual(calls, [['native part'], []]);
  } finally { delete shared.__drillPartEffects; }
});

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

test('negative and zero exact rows activate native IDs through keyboard-focusable buttons without amount geometry', () => {
  const picked: string[] = [];
  const tree = DrillParts({ parts: [{ id: 'negative', label: 'Correction', value: -2, raw: -2n },
    { id: 'zero', label: 'Zero', value: 0, raw: 0n }, { id: 'refused', label: 'Refused ID', value: 5, selectable: false }],
    format: String, total: 3, positiveBars: true, onPick: id => picked.push(id),
    unattributed: { label: 'Residual', value: 1, raw: 1n } });
  const buttons = nodes(tree).filter(node => node.type === 'button');
  assert.deepEqual(buttons.map(button => button.props['aria-label']), ['Correction, -2', 'Zero, 0']);
  for (const button of buttons) {
    assert.equal(button.props.type, 'button');
    assert.equal(button.props.tabIndex, 0);
    (button.props.onClick as () => void)();
  }
  assert.deepEqual(picked, ['negative', 'zero']);
  assert.match(renderToStaticMarkup(tree), /width:0%/);
  assert.equal(nodes(DrillParts({ parts: [{ id: 'plain', label: 'Plain', value: 0 }], format: String }))
    .some(node => node.type === 'button'), false);
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

test('country rows retain missing and pending flag frames while generic part artwork stays optional', () => {
  const html = renderToStaticMarkup(DrillParts({ parts: [
    { id: 'missing', label: 'Missing country', value: 2, countryFlag: { flagReason: 'Flag read failed at this hour' } },
    { id: 'pending', label: 'Pending country', value: 1, countryFlag: { flagPending: true } },
    { id: 'loaded', label: 'Loaded country', value: 0, countryFlag: { flag: 'dated.png' } },
    { id: 'equipment', label: 'Equipment', value: -1, icon: 'equipment.png' },
    { id: 'residual', label: 'Not identified', value: null },
  ], format: String }));
  assert.equal((html.match(/country-flag-unavailable/g) ?? []).length, 2);
  assert.match(html, /Flag unavailable/);
  assert.match(html, /Flag loading/);
  assert.match(html, /class="country-flag-art drill-part-flag" src="dated.png"/);
  assert.match(html, /<img src="equipment.png" alt=""/);
  assert.doesNotMatch(html, /No artwork/);
});
