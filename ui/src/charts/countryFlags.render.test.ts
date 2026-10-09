import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
const modules = await Promise.all([
  vite.ssrLoadModule('/ui/src/charts/table/Table.tsx'),
  vite.ssrLoadModule('/ui/src/charts/TooltipLayer.tsx'),
  vite.ssrLoadModule('/ui/src/charts/tooltip.ts'),
]).finally(() => vite.close());
const { CountryCell } = modules[0] as typeof import('./table/Table.tsx');
const { default: TooltipLayer } = modules[1] as typeof import('./TooltipLayer.tsx');
const { showTooltip, hideTooltip, tooltipText, tooltipStore } = modules[2] as typeof import('./tooltip.ts');

test('completed unavailable country flags retain their provided reason and an authoritative empty name', () => {
  const unavailable = { name: '', flagReason: 'Synthetic flag definition was refused', flagPending: false };
  const markup = renderToStaticMarkup(createElement(CountryCell, unavailable));
  assert.match(markup, /country-flag-unavailable/);
  assert.match(markup, /aria-label="Flag not available: Synthetic flag definition was refused"/);
  assert.match(markup, /title=""/);
  assert.doesNotMatch(markup, /Flag loading|Flag not in this recording|<img/);
  const pending = renderToStaticMarkup(createElement(CountryCell, { name: 'Current actor', flagPending: true }));
  assert.match(pending, /aria-label="Flag loading"/);
  assert.doesNotMatch(pending, /Flag not available/);
});

test('country tooltip failures stay visible and spoken while generic artwork and unknown actors remain flagless', () => {
  const content = { title: 'Actual dated actor', countryFlag: { flagReason: 'Synthetic header flag refused', flagPending: false },
    rows: [{ label: 'Known country', countryFlag: { flagReason: 'Synthetic flag request failed', flagPending: false } },
      { label: 'Native participant reference not recorded' }, { label: 'Generic artwork', icon: undefined }] };
  showTooltip('flags-render', content, 10, 20);
  try {
    const markup = renderToStaticMarkup(createElement(TooltipLayer));
    assert.equal((markup.match(/country-flag-unavailable/g) ?? []).length, 2);
    assert.match(markup, /<small>Flag not available: Synthetic flag request failed<\/small>/);
    assert.doesNotMatch(markup, /Flag loading/);
    assert.match(tooltipText(content), /Synthetic header flag refused/);
    assert.match(tooltipText(content), /Synthetic flag request failed/);
    assert.match(markup, /Native participant reference not recorded/);
  } finally { hideTooltip('flags-render'); }
});

test('country tooltip pending and missing states without backend reasons are visible and spoken', () => {
  const content = { title: 'Pending country', countryFlag: { flagPending: true },
    rows: [{ label: 'Known missing country', countryFlag: {} }, { label: 'Unknown native actor' }, { label: 'Generic missing artwork' }] };
  showTooltip('flags-status', content, 10, 20);
  try {
    const markup = renderToStaticMarkup(createElement(TooltipLayer));
    assert.match(markup, /<small>Flag loading<\/small>/);
    assert.match(markup, /<small>Flag not available<\/small>/);
    assert.equal((markup.match(/country-flag-unavailable/g) ?? []).length, 2);
    assert.equal(tooltipText(content), 'Pending country; Flag loading; Known missing country Flag not available; Unknown native actor; Generic missing artwork');
    assert.equal(tooltipText({ ...content, spoken: 'Caller supplied complete description' }), 'Caller supplied complete description');
  } finally { hideTooltip('flags-status'); }
});

test('a complete scrollable native list mounts no hidden flag images before its viewport is measured', () => {
  const content = { title: 'Full native list', scrollable: true, scrollId: 'native-item',
    rows: Array.from({ length: 5000 }, (_, index) => ({ label: `Native country ${index}`,
      countryFlag: { flag: `synthetic-row-image-${index}` } })) };
  showTooltip('flags-scroll', content, 10, 20);
  try {
    const markup = renderToStaticMarkup(createElement(TooltipLayer));
    assert.equal(tooltipStore.get()!.content.rows.length, 5000);
    assert.match(markup, /chart-tooltip is-scrollable/);
    assert.match(markup, /chart-tooltip-rows/);
    assert.doesNotMatch(markup, /synthetic-row-image-/);
    assert.match(tooltipText(content), /Native country 4999/);
  } finally { hideTooltip('flags-scroll'); }
});
