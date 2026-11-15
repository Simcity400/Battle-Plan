import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import { createServer } from 'vite';

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
const { default: ProgressMeter } = await vite.ssrLoadModule('/ui/src/charts/meter/ProgressMeter.tsx').finally(() => vite.close()) as typeof import('./ProgressMeter.tsx');

test('live meters animate only when their consumer opts into a sweep', () => {
  const plain = renderToStaticMarkup(createElement(ProgressMeter, { value: .5, state: 'live' }));
  const swept = renderToStaticMarkup(createElement(ProgressMeter, { value: .5, state: 'live', sweep: true }));
  assert.match(plain, /class="progress-meter is-live"/);
  assert.match(swept, /class="progress-meter is-live has-sweep"/);
  const css = readFileSync(new URL('./meter.css', import.meta.url), 'utf8');
  const liveRules = css.match(/[^{}]*\.is-live[^{}]*\{[^{}]*\}/g)!;
  assert.equal(liveRules.length, 3);
  for (const rule of liveRules) assert.match(rule.split('{')[0], /\.is-live\.has-sweep/);
  assert.match(liveRules[2], /data-motion='reduced'.*content:none/);
});

test('vertical meters fill downward and place their milestones along the track', () => {
  const html = renderToStaticMarkup(createElement(ProgressMeter, { value: .25, state: 'live', orientation: 'vertical',
    marks: [{ id: 'a', at: .1, reached: true, label: 'First' }, { id: 'b', at: .8, reached: false, label: 'Second' }] }));
  assert.match(html, /class="progress-meter is-live is-vertical"/);
  assert.match(html, /style="height:25%"/);
  assert.match(html, /class="progress-meter-mark is-reached" style="top:10%"/);
  assert.match(html, /class="progress-meter-mark" style="top:80%"/);
  assert.match(html, /<span>Second<\/span>/);
});

test('an unknown meter keeps its milestones and draws no fill', () => {
  const html = renderToStaticMarkup(createElement(ProgressMeter, { value: null, state: 'idle', orientation: 'vertical',
    marks: [{ id: 'a', at: 2, reached: false }] }));
  assert.match(html, /aria-valuetext="Not recorded"/);
  assert.doesNotMatch(html, /<i style/);
  assert.match(html, /style="top:100%"/);
});
