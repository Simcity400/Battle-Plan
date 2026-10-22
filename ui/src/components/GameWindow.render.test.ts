import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { Scene } from '../generated/dashboard';

test('a scene shows its painted image, clips hover areas, explains them on hover and describes itself as read-only', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { SceneView } = await vite.ssrLoadModule('/ui/src/components/GameWindow.tsx') as { SceneView: unknown };
    const scene: Scene = {
      name: 'synthetic_window', size: [400, 300], viewport: [1920, 1080], image: 'data:image/png;base64,AA',
      hovers: [{ rect: [5, 6, 50, 20], title: 'Westmarch Works', details: ['Key: westmarch_works'] },
        { rect: [25, 35, 10, 10], clip: [20, 30, 120, 80], title: 'Tooling', details: [] }],
      lists: [], notes: [],
    };
    const html = renderToStaticMarkup(createElement(SceneView as never, { scene, title: 'Policies', zoom: 0.5,
      offsetOf: (_path: string, drawn: number) => drawn, onScroll: () => {} } as never));
    // The game's drawing, as the service painted it: one image, never recoloured or muted here.
    assert.equal(html.match(/<img class="game-window-image" src="data:image\/png;base64,AA" alt=""/g)?.length, 1);
    assert.doesNotMatch(html, /grayscale|game-window-muted|game-window-quad/);
    assert.match(html, /aria-label="Policies"/);
    assert.match(html, /zoom:0\.5/);
    assert.match(html, /width:400px;height:300px/, 'drawn at its own interface size');
    assert.match(html, /class="game-window-clip" style="left:20px;top:30px;width:100px;height:50px"/);
    assert.match(html, /class="game-window-origin" style="left:-20px;top:-30px"/);
    assert.equal(html.match(/class="game-window-clip"/g)?.length, 1, 'one clipped hover run');
    // Rows are focusable list items whose names carry every detail the tooltip shows.
    assert.match(html, /<div class="game-window-layer is-hovers" role="list">/);
    assert.match(html, /role="listitem" tabindex="0" aria-label="Westmarch Works. Key: westmarch_works"[^>]* data-tooltip="Westmarch Works\nKey: westmarch_works"/);
    assert.match(html, /role="listitem" tabindex="0" aria-label="Tooling"/);
    // That the window is read-only is described to assistive technology, never drawn.
    const described = html.match(/aria-describedby="([^"]+)"/)?.[1];
    assert.ok(described, 'the scene names its description');
    assert.match(html, new RegExp(`<span id="${described}" class="sr-only">Read-only: [^<]+</span>`));
    const visible = html.replace(/<span[^>]*class="sr-only"[^>]*>[^<]*<\/span>/, '').replace(/<[^>]*>/g, '');
    assert.equal(visible, '', 'no caption: names and codes appear on hover only');
  } finally { await vite.close(); }
});

test('a closed window popover is only its trigger and asks for nothing', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { default: GameWindowPopover } = await vite.ssrLoadModule('/ui/src/components/GameWindowPopover.tsx') as { default: unknown };
    let asked = 0;
    const client = { product: () => { asked++; return new Promise(() => {}); } };
    const html = renderToStaticMarkup(createElement(GameWindowPopover as never, { client, source: { sourceId: 's', revision: 'r' }, branchId: '1', hour: 10,
      target: { kind: 'mioPolicies', organisation: '7' }, label: 'Show Westmarch Works’s policy window', heading: 'Westmarch Works',
      children: createElement('small', null, 'Westmarch Works') } as never));
    assert.match(html, /<button type="button" popovertarget="[^"]+" aria-haspopup="dialog" aria-expanded="false" aria-label="Show Westmarch Works’s policy window"><small>Westmarch Works<\/small><\/button>/i);
    assert.match(html, /aria-label="Close Westmarch Works"/, 'an in-game name keeps its case');
    assert.doesNotMatch(html, /game-window"|Loading window/, 'the window is built only once opened');
    assert.equal(asked, 0);
  } finally { await vite.close(); }
});
