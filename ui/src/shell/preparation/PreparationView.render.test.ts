import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { PreparationReply } from './contract.ts';
test('preparation renders backend section names, exact progress, cancellation and Clear with bounded rows', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { PreparationView } = await vite.ssrLoadModule('/ui/src/shell/preparation/PreparationView.tsx');
    const reply: PreparationReply = { sourceId: 's', revision: 'r', requestId: 'q', state: 'preparing', error: null, backgroundEnabled: true, bytes: '9007199254740993',
      sections: Array.from({ length: 5000 }, (_, index) => ({ id: `section-${index}`, label: `Mod section ${index}`, state: 'preparing', done: '9007199254740993', total: '18014398509481986', error: null })) };
    const render = (value: PreparationReply) => renderToStaticMarkup(createElement(PreparationView, { name: 'Synthetic campaign', reply: value, loading: false, pending: false, error: null, onCommand() {} }));
    const html = render(reply);
    assert.match(html, /Mod section 0/); assert.doesNotMatch(html, /Mod section 4999/);
    assert.equal((html.match(/role="listitem"/g) ?? []).length, 5);
    assert.match(html, /aria-valuenow="50"/); assert.match(html, /9007199254740993 of 18014398509481986 completed/);
    assert.match(html, /Cancel preparation/); assert.match(html, /Clear prepared data/);
    assert.match(html, /Clear removes prepared data immediately/); assert.match(html, /aria-setsize="5000"/);
    assert.doesNotMatch(render({ ...reply, state: 'cancelled' }), /Cancel preparation/);
    assert.match(render({ ...reply, state: 'cancelled' }), /Resume preparation/);
    const paused = render({ ...reply, state: 'paused', pauseReason: 'diskSpace' });
    assert.match(paused, /role="status">Paused</); assert.match(paused, /Paused because less than a tenth of the disk holding prepared data is free\. It resumes automatically\./);
    assert.match(render({ ...reply, state: 'paused', pauseReason: 'requested' }), /while preparation you asked for runs/);
    assert.match(paused, /Resume preparation/); assert.match(paused, /Cancel preparation/);
    assert.match(render({ ...reply, state: 'failed', error: 'Unsupported mod section: 3' }), /role="alert"[^>]*>Unsupported mod section: 3/);
  } finally { await vite.close(); }
});
