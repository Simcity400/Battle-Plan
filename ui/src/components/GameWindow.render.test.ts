import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { Scene, SceneQuad, WindowTextureImage } from '../generated/dashboard';

type Point = [number, number];
const rect = (x0: number, y0: number, x1: number, y1: number): [Point, Point, Point, Point] => [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
const quad = (texture: number, clip?: number): SceneQuad =>
  ({ texture, corners: rect(0, 0, 10, 10), uv: rect(0, 0, 1, 1), color: [1, 1, 1, 1], ...(clip === undefined ? {} : { clip }) });

test('a scene draws loaded pages only, clips its runs and explains hover areas on hover', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { SceneView } = await vite.ssrLoadModule('/ui/src/components/GameWindow.tsx') as { SceneView: unknown };
    const scene: Scene = {
      name: 'synthetic_window', size: [400, 300], viewport: [1920, 1080],
      textures: [{ id: 1, width: 10, height: 10 }, { id: 2, width: 10, height: 10 }],
      clips: [[20, 30, 120, 80]],
      quads: [quad(1), quad(2), quad(1, 0), { ...quad(1, 0), muted: true }, quad(1)],
      hovers: [{ rect: [5, 6, 50, 20], title: 'Westmarch Works', details: ['Key: westmarch_works'] },
        { rect: [25, 35, 10, 10], clip: 0, title: 'Tooling', details: [] }],
      lists: [], effects: [], notes: [],
    };
    const images = new Map<number, WindowTextureImage>([[1, { id: 1, width: 10, height: 10, image: 'data:image/png;base64,AA' }],
      [2, { id: 2, width: 10, height: 10, unavailable: 'Not readable' }]]);
    const html = renderToStaticMarkup(createElement(SceneView as never, { scene, title: 'Policies', images, tints: new Map(), version: 1, zoom: 0.5,
      offsetOf: (_path: string, drawn: number) => drawn, onScroll: () => {} } as never));
    assert.equal(html.match(/class="game-window-quad"/g)?.length, 4, 'the page without an image leaves its quad undrawn');
    assert.match(html, /<div class="game-window-muted"><div class="game-window-quad"[^>]*><\/div><\/div>/, 'a muted run is muted as one group');
    assert.match(html, /aria-label="Policies"/);
    assert.match(html, /zoom:0\.5/);
    assert.match(html, /width:400px;height:300px/, 'drawn at its own interface size');
    assert.match(html, /class="game-window-clip" style="left:20px;top:30px;width:100px;height:50px"/);
    assert.match(html, /class="game-window-origin" style="left:-20px;top:-30px"/);
    assert.equal(html.match(/class="game-window-clip"/g)?.length, 3, 'two clipped quad runs, one clipped hover run');
    // Rows are focusable list items whose names carry every detail the tooltip shows.
    assert.match(html, /<div class="game-window-layer is-hovers" role="list">/);
    assert.match(html, /role="listitem" tabindex="0" aria-label="Westmarch Works. Key: westmarch_works"[^>]* data-tooltip="Westmarch Works\nKey: westmarch_works"/);
    assert.match(html, /role="listitem" tabindex="0" aria-label="Tooling"/);
    const visible = html.replace(/<[^>]*>/g, '');
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
