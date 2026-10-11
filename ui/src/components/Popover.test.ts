import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { HOUR_EPOCH } from '../dashboard/calendar.ts';

test('the header pop-downs build their contents only once opened', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { default: DateTimePicker } = await vite.ssrLoadModule('/ui/src/components/DateTimePicker.tsx') as typeof import('./DateTimePicker.tsx');
    const { default: PlaybackSpeed } = await vite.ssrLoadModule('/ui/src/components/PlaybackSpeed.tsx') as typeof import('./PlaybackSpeed.tsx');
    const date = renderToStaticMarkup(createElement(DateTimePicker, { label: 'Date', value: HOUR_EPOCH + 100, min: HOUR_EPOCH, max: HOUR_EPOCH + 24 * 365,
      otherValue: null, onChange() {} }));
    assert.match(date, /class="ui-popover date-time-popover"/);
    assert.doesNotMatch(date, /calendar-day|First date/, 'a closed calendar renders no days');
    const speed = renderToStaticMarkup(createElement(PlaybackSpeed, { value: 24, onChange() {} }));
    assert.match(speed, /class="ui-popover playback-speed-menu"/);
    assert.doesNotMatch(speed, /Custom speed/, 'a closed speed menu renders no choices');
  } finally { await vite.close(); }
});
