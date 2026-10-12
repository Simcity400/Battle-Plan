import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { raceHeight } from './race.ts';

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
const { default: RaceChart } = await vite.ssrLoadModule('/ui/src/charts/race/RaceChart.tsx').finally(() => vite.close()) as typeof import('./RaceChart.tsx');

test('a race scrolls as far as its entries, not the empty slots kept for the top N', () => {
  const items = ['a', 'b', 'c'].map((key, index) => ({ key, value: 3 - index, label: String(3 - index), segments: [], stretch: 0 }));
  const html = renderToStaticMarkup(createElement(RaceChart, { label: 'Territory', items, slots: 30, name: key => key, active: true }));
  assert.match(html, /class="race-scroll[^"]*"/);
  assert.match(html, new RegExp(`class="race-extent" style="height:calc\\(${raceHeight(3)}px - 100%\\)"`));
});
