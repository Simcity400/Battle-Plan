import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetStateProps } from './WidgetState';

test('an error says what failed to load and offers Retry, never the underlying message', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { default: WidgetState } = await vite.ssrLoadModule('/ui/src/components/WidgetState.tsx') as { default: (props: WidgetStateProps) => unknown };
    const render = (props: WidgetStateProps) => renderToStaticMarkup(createElement(WidgetState as never, props as never));
    const whole = render({ kind: 'error', detail: 'The unit leaders reply is malformed.', onRetry: () => {} });
    const visible = whole.replace(/<[^>]*>/g, ' ');
    assert.match(visible, /Couldn’t load\./);
    assert.match(visible, /Retry/);
    assert.doesNotMatch(visible, /malformed/, 'the detail is not shown');
    assert.match(whole, /data-detail="The unit leaders reply is malformed\."/, 'developer tools still see it');
    const part = render({ kind: 'error', what: 'faction members' }).replace(/<[^>]*>/g, ' ');
    assert.match(part, /Couldn’t load faction members\./);
    assert.doesNotMatch(part, /Retry/, 'no Retry without a way to retry');
  } finally { await vite.close(); }
});
