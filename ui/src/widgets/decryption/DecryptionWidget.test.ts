import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import { createServer } from 'vite';
import type { DecryptionReply } from '../../generated/dashboard';
import type { TooltipContent } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';

const data: DecryptionReply = { branchId: '1', warnings: [], coverage: [{ firstHour: 100, lastHour: 100, complete: true, reason: null }],
  pairs: [0, 1].map(index => ({ ownerId: String(index * 2 + 1), targetId: String(index * 2 + 2),
    tagRuns: [{ firstHour: 100, lastHour: 100, ownerTag: `Owner ${index}`, targetTag: `Target ${index}` }],
    runs: [{ firstHour: 100, lastHour: 100, startAmountRaw: '400000', stepRawPerDay: '0', thresholdRaw: '800000', thresholdUnknown: null,
      bonusDays: 3, bonusDaysStepPerDay: 0, stateFlags: 0x100, broken: true, started: { hour: 99, upperBound: true } }],
    breaks: [{ hour: 100, upperBound: true }] })) };
const host = globalThis as typeof globalThis & { __intelTooltipCalls?: { owner: string; content: (key: string) => TooltipContent | null }[] };
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'recorded-cipher-fixture', enforce: 'pre',
    transform(source, id) {
      if (id.endsWith('/dashboard/products.ts')) return source.slice(0, source.indexOf('export function useProduct<T>'))
        + `export function useProduct() { return { data: ${JSON.stringify(data)}, retry: () => {} }; }`;
      if (id.endsWith('/decryption/decryptionCountries.ts')) return source.slice(0, source.indexOf('export function useDecryptionCountries'))
        + 'export function useDecryptionCountries() { return { pair: row => ({ owner: { name: `Recorded ${row.tags.ownerTag}` }, target: { name: `Recorded ${row.tags.targetTag}` }, hour: row.hour }), revision: 1 }; }';
      if (id.endsWith('/components/ContextMenu.tsx')) return source.slice(0, source.indexOf('export function useContextMenu'))
        + 'export function useContextMenu() { return { menu: null, open: () => {} }; }';
      if (id.endsWith('/charts/useRowTooltips.ts')) return source.replace('  const [hovered, setHovered]',
        '  globalThis.__intelTooltipCalls?.push({ owner, content });\n  const [hovered, setHovered]');
    },
  }] });
const { default: DecryptionWidget } = await vite.ssrLoadModule('/ui/src/widgets/decryption/DecryptionWidget.tsx').finally(() => vite.close()) as typeof import('./DecryptionWidget.tsx');

const props = { widget: { id: 'cipher-list', kind: 'decryption', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} },
  runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 100 } }],
    clock: { getHour: () => 100, subscribeHour: () => () => {} }, client: {} }, active: true, onDiagnostics: () => {} } as unknown as WidgetProps;

test('one cipher-list tooltip owner serves both rows and their accessible descriptions', () => {
  host.__intelTooltipCalls = [];
  try {
    const html = renderToStaticMarkup(createElement(DecryptionWidget, props));
    assert.equal(host.__intelTooltipCalls.length, 1);
    const tips = host.__intelTooltipCalls[0];
    assert.equal(tips.owner, 'decryption:cipher-list');
    for (const [index, pair] of data.pairs.entries()) assert.equal(tips.content(JSON.stringify([pair.ownerId, pair.targetId]))!.title,
      `Recorded Owner ${index} → Recorded Target ${index}`);
    const ids = [...html.matchAll(/aria-describedby="([^"]+)"/g)].map(match => match[1]);
    assert.equal(ids.length, 2);
    for (const id of ids) assert.ok(html.includes(`id="${id}" class="sr-only"`));
    assert.match(html, /class="pill warning">\+3 bonus days/);
    assert.match(html, /progress-meter is-done has-sweep/);
  } finally { delete host.__intelTooltipCalls; }
});

test('cipher layout uses shared sizes and typography without a duplicate badge style', () => {
  const css = readFileSync(new URL('./decryption.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /\b\d+(?:\.\d+)?px\b|font-size:\s*calc\(|decryption-badge|badge-warning-background/);
  assert.match(css, /decryption-unknown[^}]*font-size: var\(--type-xs\)/);
  assert.match(css, /decryption-row[^}]*height: 100%/);
});
