import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('a widget that throws shows its own error with Retry instead of unmounting the app', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    type Boundary = { getDerivedStateFromError(error: unknown): { error: Error } } & (new (props: { children: unknown; reset: unknown }) => {
      state: { error: Error | null }; props: { reset: unknown }; render(): unknown; setState(state: unknown): void });
    const { default: WidgetBoundary } = await vite.ssrLoadModule('/ui/src/dashboard/WidgetBoundary.tsx') as { default: Boundary };
    const boundary = new WidgetBoundary({ children: createElement('span', null, 'chart'), reset: 1 });
    assert.equal(renderToStaticMarkup(boundary.render() as never), '<span>chart</span>');
    boundary.state = WidgetBoundary.getDerivedStateFromError(new Error('Maximum update depth exceeded.'));
    const visible = renderToStaticMarkup(boundary.render() as never);
    assert.match(visible, /Couldn’t load this widget\./);
    assert.match(visible, /Retry/);
    assert.match(visible, />Details</);
    assert.match(visible, /data-detail="Maximum update depth exceeded\."/);
    assert.match(visible, /role="alert"/);
    assert.doesNotMatch(visible.replace(/<[^>]*>/g, ' '), /Maximum update depth exceeded\./);
    assert.equal(WidgetBoundary.getDerivedStateFromError('thrown text').error.message, 'thrown text');
  } finally { await vite.close(); }
});
