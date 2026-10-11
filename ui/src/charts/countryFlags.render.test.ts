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

test('a completed unavailable country flag keeps its reason as metadata only, and an authoritative empty name', () => {
  const unavailable = { name: '', flagReason: 'Synthetic flag definition was refused', flagPending: false };
  const markup = renderToStaticMarkup(createElement(CountryCell, unavailable));
  assert.match(markup, /country-flag-unavailable/);
  assert.match(markup, /aria-label="Flag unavailable"/);
  assert.doesNotMatch(markup, /Synthetic flag definition/, 'the reason is never shown or read out');
  assert.match(markup, /title=""/);
  assert.doesNotMatch(markup, /Flag loading|Flag not in this recording|<img/);
  const pending = renderToStaticMarkup(createElement(CountryCell, { name: 'Current actor', flagPending: true }));
  assert.match(pending, /aria-label="Flag loading"/);
  assert.doesNotMatch(pending, /Flag unavailable/);
});

test('country tooltip flag failures show only the neutral placeholder, never a reason, while generic artwork and unknown actors remain flagless', () => {
  const content = { title: 'Actual dated actor', countryFlag: { flagReason: 'Synthetic header flag refused', flagPending: false },
    rows: [{ label: 'Known country', countryFlag: { flagReason: 'Synthetic flag request failed', flagPending: false } },
      { label: 'Native participant reference not recorded' }, { label: 'Generic artwork', icon: undefined }] };
  showTooltip('flags-render', content, 10, 20);
  try {
    const markup = renderToStaticMarkup(createElement(TooltipLayer));
    assert.equal((markup.match(/country-flag-unavailable/g) ?? []).length, 2);
    assert.doesNotMatch(markup, /Synthetic/, 'no reason, even in labels');
    assert.doesNotMatch(markup.replace(/<[^>]*>/g, ' '), /Flag/, 'the placeholder adds no visible text');
    assert.equal(tooltipText(content), 'Actual dated actor; Known country; Native participant reference not recorded; Generic artwork');
    assert.match(markup, /Native participant reference not recorded/);
  } finally { hideTooltip('flags-render'); }
});

test('pending and missing country flags add no text to a tooltip or what it reads out', () => {
  const content = { title: 'Pending country', countryFlag: { flagPending: true },
    rows: [{ label: 'Known missing country', countryFlag: {} }, { label: 'Unknown native actor' }, { label: 'Generic missing artwork' }] };
  showTooltip('flags-status', content, 10, 20);
  try {
    const markup = renderToStaticMarkup(createElement(TooltipLayer));
    assert.doesNotMatch(markup.replace(/<[^>]*>/g, ' '), /Flag/, 'the placeholders add no visible text');
    assert.equal((markup.match(/country-flag-unavailable/g) ?? []).length, 2);
    assert.equal(tooltipText(content), 'Pending country; Known missing country; Unknown native actor; Generic missing artwork');
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
