import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('Follow uses native button activation, announces its state and disables unavailable pause', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { default: Follow } = await vite.ssrLoadModule('/ui/src/charts/drill/FollowControl.tsx');
    let follows = 0, pauses = 0;
    const props = { following: false, onFollow: () => follows++, onPause: () => pauses++ };
    const button = () => Follow(props);
    assert.equal(button().type, 'button', 'Enter and Space use browser button activation');
    assert.equal(button().props.type, 'button');
    assert.equal(button().props['aria-label'], 'Follow playback');
    assert.equal(button().props['aria-pressed'], false);
    assert.equal(button().props.disabled, false);
    button().props.onClick(); assert.equal(follows, 1);
    props.following = true;
    assert.equal(button().props['aria-pressed'], true);
    assert.equal(button().props.disabled, false);
    button().props.onClick(); assert.equal(pauses, 1); assert.equal(follows, 1);
    const locked = { following: true, onFollow: props.onFollow };
    const html = renderToStaticMarkup(createElement(Follow, locked));
    assert.match(html, /aria-pressed="true" disabled=""/);
    assert.match(html, />Follow<\/button>/);
    assert.doesNotMatch(html, /Following playback|data-tooltip/);
    locked.following = false;
    assert.equal(Follow(locked).props.disabled, false, 'returning to follow remains available without pause plumbing');
    Follow(locked).props.onClick(); assert.equal(follows, 2);
  } finally { await vite.close(); }
});
