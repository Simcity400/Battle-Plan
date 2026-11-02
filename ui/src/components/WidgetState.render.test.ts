import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetStateProps } from './WidgetState';

test('technical errors stay generic with Retry and named refusals have no Retry', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { queryFailure } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    const { default: WidgetState } = await vite.ssrLoadModule('/ui/src/components/WidgetState.tsx') as { default: (props: WidgetStateProps) => unknown };
    const render = (props: WidgetStateProps) => renderToStaticMarkup(createElement(WidgetState as never, props as never));
    const whole = render({ kind: 'error', failure: queryFailure({ kind: 'failed', message: 'The unit leaders reply is malformed.' }), onRetry: () => {} });
    const visible = whole.replace(/<[^>]*>/g, ' ');
    assert.match(visible, /Couldn’t load\./);
    assert.match(visible, /Retry/);
    assert.doesNotMatch(visible, /malformed/, 'the detail is not shown');
    assert.match(whole, /data-detail="The unit leaders reply is malformed\."/, 'developer tools still see it');
    const part = render({ kind: 'error', what: 'faction members' }).replace(/<[^>]*>/g, ' ');
    assert.match(part, /Couldn’t load faction members\./);
    assert.doesNotMatch(part, /Retry/, 'no Retry without a way to retry');
    const withheld = render({ kind: 'unavailable', message: '2 leaders can’t be shown.', detail: 'Commander: Template missing\nLeader 2: Owner missing' });
    assert.match(withheld, /2 leaders can’t be shown\./);
    assert.match(withheld, /<small>Commander: Template missing\nLeader 2: Owner missing<\/small>/);
    const refused = render({ kind: 'error', failure: queryFailure({ kind: 'refused', text: 'Land owners are not prepared natively yet' }), onRetry: () => {} }).replace(/<[^>]*>/g, ' ');
    assert.match(refused, /Land owners are not prepared natively yet/);
    assert.doesNotMatch(refused, /Retry/);
    assert.doesNotMatch(refused, /Couldn’t load/);
    for (const kind of ['empty', 'unavailable'] as const) {
      const message = kind === 'empty' ? 'No generals yet.' : 'Leaders weren’t recorded at this time.';
      const detail = createElement('span', { 'data-detail': 'Synthetic country styles decoder failed' }, 'Couldn’t load country name.');
      const html = render({ kind, message, detail });
      assert.ok(html.includes(`<span>${message}</span>`));
      assert.ok(html.includes('<small><span data-detail="Synthetic country styles decoder failed">Couldn’t load country name.</span></small>'));
      assert.doesNotMatch(html, /is-error|role="alert"/);
    }
  } finally { await vite.close(); }
});
