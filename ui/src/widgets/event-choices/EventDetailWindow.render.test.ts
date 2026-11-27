import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { EventDetailSubject, EventDetailWindowProps } from './EventDetailWindow.tsx';

test('the event detail distinguishes a missing firing and an observed choice from a recorded firing', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { default: EventDetailWindow } = await vite.ssrLoadModule('/ui/src/widgets/event-choices/EventDetailWindow.tsx') as { default: (props: EventDetailWindowProps) => unknown };
    const render = (firedHour: number | null, chosenOrdinal: number | null = 0) => {
      const subject: EventDetailSubject = { key: 'orphan', hour: 137, firedHour, asOfHour: 150, title: 'A synthetic event',
        countryTag: null, countryName: null, item: null, chosenOrdinal, awaiting: false, chosenHour: 137, noChoiceReason: 'Chosen option not recorded' };
      return renderToStaticMarkup(createElement(EventDetailWindow as never, { subject, anchor: { current: null }, onClose() {} } as never));
    };
    const orphan = render(null);
    assert.match(orphan, /Firing date not recorded/);
    assert.match(orphan, /title="The event firing was not recorded/);
    assert.match(orphan, /Chosen by /);
    assert.doesNotMatch(orphan, /Fired /);
    const unidentified = render(null, null);
    assert.match(unidentified, /Chosen by /);
    assert.match(unidentified, /Chosen option not recorded/);
    const recorded = render(119);
    assert.match(recorded, /Fired /);
    assert.doesNotMatch(recorded, /Firing date not recorded|Chosen by /);
  } finally {
    await vite.close();
  }
});
