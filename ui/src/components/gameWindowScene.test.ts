import assert from 'node:assert/strict';
import test from 'node:test';
import type { SceneQuad } from '../generated/dashboard';
import { MIN_ZOOM, batches, clipRuns, described, fitZoom, keyedOffset, listAt, revealedOffset, tintName, tintPixels, tinted, missingTextures, quadStyle, scrolledOffset, wheelPixels } from './gameWindowScene.ts';

type Point = [number, number];
const quad = (corners: [Point, Point, Point, Point], uv: [Point, Point, Point, Point], extra: Partial<SceneQuad> = {}): SceneQuad =>
  ({ texture: 1, corners, uv, color: [1, 1, 1, 1], ...extra });
const rect = (x0: number, y0: number, x1: number, y1: number): [Point, Point, Point, Point] => [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
const PAGE = { width: 64, height: 32 };

/** Where the element's texel point lands under the style's transform. */
function land(style: { transform?: unknown }, x: number, y: number): Point {
  const [a, b, c, d, e, f] = String(style.transform).replace(/^matrix\(|\)$/g, '').split(',').map(Number);
  return [a * x + c * y + e, b * x + d * y + f];
}
const close = (actual: Point, expected: Point) => {
  assert.ok(Math.abs(actual[0] - expected[0]) < 1e-9 && Math.abs(actual[1] - expected[1]) < 1e-9, `${actual} ≈ ${expected}`);
};

test('a quad shows its texture rectangle at native texel size, placed on its corners', () => {
  const style = quadStyle(quad(rect(10, 20, 26, 28), rect(0.25, 0.5, 0.5, 0.75)), PAGE, 'data:image/png;base64,AA')!;
  assert.equal(style.width, '16px');
  assert.equal(style.height, '8px');
  assert.equal(style.backgroundImage, 'url("data:image/png;base64,AA")');
  assert.equal(style.backgroundSize, '64px 32px');
  assert.equal(style.backgroundRepeat, 'repeat');
  assert.equal(style.backgroundPosition, '-16px -16px');
  assert.equal(style.transformOrigin, '0 0');
  close(land(style, 0, 0), [10, 20]);
  close(land(style, 16, 0), [26, 20]);
  close(land(style, 16, 8), [26, 28]);
  close(land(style, 0, 8), [10, 28]);
  assert.equal(style.opacity, undefined, 'an untinted opaque quad needs no colour');
  assert.equal(style.maskImage, undefined);
});

test('a quad is stretched, mirrored and rotated exactly onto its corners', () => {
  // Stretched to twice its texels.
  const stretched = quadStyle(quad(rect(0, 0, 64, 32), rect(0, 0, 0.5, 0.5)), PAGE, 'u')!;
  close(land(stretched, 32, 16), [64, 32]);
  // Mirrored horizontally: the texture's right edge lands on the left corner.
  const mirrored = quadStyle(quad(rect(0, 0, 32, 16), [[0.5, 0], [0, 0], [0, 0.5], [0.5, 0.5]]), PAGE, 'u')!;
  assert.equal(mirrored.width, '32px');
  assert.equal(mirrored.backgroundPosition, '0px 0px');
  close(land(mirrored, 32, 0), [0, 0]);
  close(land(mirrored, 0, 0), [32, 0]);
  close(land(mirrored, 0, 16), [32, 16]);
  // Rotated a quarter turn.
  const rotated = quadStyle(quad([[100, 0], [100, 64], [68, 64], [68, 0]], rect(0, 0, 1, 1)), PAGE, 'u')!;
  close(land(rotated, 0, 0), [100, 0]);
  close(land(rotated, 64, 0), [100, 64]);
  close(land(rotated, 64, 32), [68, 64]);
  close(land(rotated, 0, 32), [68, 0]);
});

test('coordinates beyond the page repeat it from the rectangle’s own origin', () => {
  const tiled = quadStyle(quad(rect(0, 0, 192, 32), rect(1, 0, 4, 1)), PAGE, 'u')!;
  assert.equal(tiled.width, '192px');
  assert.equal(tiled.backgroundPosition, '-64px 0px');
  close(land(tiled, 192, 32), [192, 32]);
});

test('a quad covering no area is not drawn', () => {
  assert.equal(quadStyle(quad(rect(0, 0, 10, 10), rect(0.5, 0, 0.5, 1)), PAGE, 'u'), null);
  assert.equal(quadStyle(quad(rect(0, 0, 10, 0), rect(0, 0, 1, 1)), PAGE, 'u'), null);
  assert.equal(quadStyle(quad([[0, 0], [10, 10], [20, 20], [5, 5]], rect(0, 0, 1, 1)), PAGE, 'u'), null);
});

test('a colour multiplies each pixel\u2019s straight colour and keeps its alpha; the quad\u2019s alpha is its opacity', () => {
  // A half-transparent texel times half: the colour halves, the alpha stays, nothing of an
  // opaque backdrop is mixed in.
  const pixels = new Uint8ClampedArray([64, 128, 192, 128, 255, 255, 255, 0, 10, 20, 30, 255]);
  tintPixels(pixels, [0.5, 0.5, 0.5]);
  assert.deepEqual([...pixels], [32, 64, 96, 128, 128, 128, 128, 0, 5, 10, 15, 255]);
  const channels = new Uint8ClampedArray([200, 200, 200, 64]);
  tintPixels(channels, [1, 0.5, 0]);
  assert.deepEqual([...channels], [200, 100, 0, 64]);
  const plain = quad(rect(0, 0, 8, 8), rect(0, 0, 0.125, 0.25), { color: [1, 1, 1, 0.5] });
  assert.equal(tinted(plain), false);
  const coloured = quad(rect(0, 0, 8, 8), rect(0, 0, 0.125, 0.25), { color: [1, 0.5, 0, 0.25], texture: 3 });
  assert.equal(tinted(coloured), true);
  assert.equal(tintName(coloured), '3|1,0.5,0');
  // The page drawn is already coloured: no blend or mask, only the alpha as opacity.
  const style = quadStyle(coloured, PAGE, 'u')!;
  assert.equal(style.backgroundBlendMode, undefined);
  assert.equal(style.maskImage, undefined);
  assert.equal(style.opacity, 0.25);
  assert.equal(quadStyle(plain, PAGE, 'u')!.opacity, 0.5);
});

test('keys scroll a list by a line, a page or to either end, within its range', () => {
  const list = { rect: [0, 0, 100, 120] as [number, number, number, number], max: 300, horizontal: false, scale: 1 };
  assert.equal(keyedOffset('ArrowDown', 0, list, 40), 40);
  assert.equal(keyedOffset('ArrowUp', 20, list, 40), 0);
  assert.equal(keyedOffset('PageDown', 100, list, 40), 220);
  assert.equal(keyedOffset('PageDown', 250, list, 40), 300);
  assert.equal(keyedOffset('PageUp', 100, list, 40), 0);
  assert.equal(keyedOffset('End', 0, list, 40), 300);
  assert.equal(keyedOffset('Home', 200, list, 40), 0);
  assert.equal(keyedOffset('Enter', 200, list, 40), null);
  // A focused row is brought just within the box: from below, from above, or left where it is.
  const box = { rect: [0, 10, 100, 100] as [number, number, number, number], max: 300, horizontal: false, scale: 1 };
  assert.equal(revealedOffset(0, box, [0, 100, 100, 30]), 20);
  assert.equal(revealedOffset(60, box, [0, -20, 100, 30]), 30);
  assert.equal(revealedOffset(60, box, [0, 40, 100, 30]), 60);
  assert.equal(revealedOffset(290, box, [0, 200, 100, 30]), 300);
  // A list that scrolls across reveals along x, and its keys are right and left.
  const across = { rect: [10, 0, 100, 40] as [number, number, number, number], max: 300, horizontal: true, scale: 1 };
  assert.equal(revealedOffset(0, across, [100, 0, 30, 40]), 20);
  assert.equal(revealedOffset(50, across, [-10, 0, 30, 40]), 30);
  assert.equal(keyedOffset('ArrowRight', 0, across, 30), 30);
  assert.equal(keyedOffset('ArrowDown', 0, across, 30), null);
  assert.equal(keyedOffset('PageDown', 0, across, 30), 100);
  // In a container at half scale, five rows of 30 drawn as 15 in a list of 100 drawn as 50: the
  // fifth row, drawn 25 pixels past the box, needs offset 50; a line of 15 pixels is 30 units, a
  // page of 50 pixels is 100.
  for (const horizontal of [false, true]) {
    const half = { rect: (horizontal ? [10, 0, 50, 15] : [0, 10, 15, 50]) as [number, number, number, number], max: 300, horizontal, scale: 0.5 };
    const fifth = horizontal ? [70, 0, 15, 15] : [0, 70, 15, 15];
    assert.equal(revealedOffset(0, half, fifth), 50);
    assert.equal(keyedOffset(horizontal ? 'ArrowRight' : 'ArrowDown', 0, half, 15), 30);
    assert.equal(keyedOffset('PageDown', 0, half, 15), 100);
  }
  assert.equal(described({ title: 'Tooling', details: ['Not available: its availability condition does not hold', 'Base attach cost (recorded): 50'] }),
    'Tooling. Not available: its availability condition does not hold. Base attach cost (recorded): 50');
});

test('clip runs group consecutive quads only, keeping draw order', () => {
  const at = (clip?: number) => quad(rect(0, 0, 1, 1), rect(0, 0, 1, 1), clip === undefined ? {} : { clip });
  const quads = [at(), at(), at(0), at(0), at(), at(1), at(0)];
  const runs = clipRuns(quads);
  assert.deepEqual(runs.map(run => [run.clip, run.start, run.quads.length]),
    [[undefined, 0, 2], [0, 2, 2], [undefined, 4, 1], [1, 5, 1], [0, 6, 1]]);
  assert.deepEqual(runs.flatMap(run => run.quads), quads);
  assert.deepEqual(clipRuns([]), []);
  // A muted row is its own run, within one clip.
  const row = (muted: boolean) => quad(rect(0, 0, 1, 1), rect(0, 0, 1, 1), muted ? { clip: 0, muted } : { clip: 0 });
  assert.deepEqual(clipRuns([row(false), row(true), row(true), row(false)]).map(run => [run.muted, run.start, run.quads.length]),
    [[false, 0, 1], [true, 1, 2], [false, 3, 1]]);
});

test('the wheel moves a list by interface pixels within its range', () => {
  assert.equal(scrolledOffset(0, 100, 1, 500), 100);
  assert.equal(scrolledOffset(0, 100, 0.5, 500), 200, 'a window drawn at half size scrolls twice as far per pixel');
  assert.equal(scrolledOffset(50, -100, 1, 500), 0);
  assert.equal(scrolledOffset(450, 100, 1, 500), 500);
  assert.equal(scrolledOffset(10, 0.4, 1, 500), 10);
  assert.equal(wheelPixels(3, 1, 400), 48);
  assert.equal(wheelPixels(1, 2, 400), 400);
  assert.equal(wheelPixels(-7, 0, 400), -7);
});

test('a window fits its whole size into the space it has, shrinking or growing', () => {
  assert.equal(fitZoom([400, 200], [320, 200]), 0.8);
  assert.equal(fitZoom([400, 200], [800, 1000]), 2);
  assert.equal(fitZoom([400, 200], [0, 0]), 1);
  // A window in a small space stops shrinking where its text stays legible, and scrolls.
  assert.equal(fitZoom([400, 200], [120, 200]), MIN_ZOOM);
});

test('the topmost list under the pointer scrolls', () => {
  const lists = [{ path: 'a', rect: [0, 0, 100, 100] as [number, number, number, number], offset: 0, max: 10, horizontal: false, scale: 1 },
    { path: 'a/b', rect: [10, 10, 20, 20] as [number, number, number, number], offset: 0, max: 10, horizontal: false, scale: 1 }];
  assert.equal(listAt({ lists }, 15, 15)?.path, 'a/b');
  assert.equal(listAt({ lists }, 50, 50)?.path, 'a');
  assert.equal(listAt({ lists }, 100, 50), null, 'the area ends before its width');
});

test('only pages not already held are requested, once each, in small batches', () => {
  const quads = [3, 1, 3, 2, 7, 1, 9, 4, 5, 6, 8].map(texture => ({ ...quad(rect(0, 0, 1, 1), rect(0, 0, 1, 1)), texture }));
  const have = new Set([2]);
  const missing = missingTextures({ quads }, id => have.has(id));
  assert.deepEqual(missing, [3, 1, 7, 9, 4, 5, 6, 8]);
  assert.deepEqual(batches(missing), [[3, 1, 7, 9, 4, 5], [6, 8]]);
  assert.deepEqual(batches([]), []);
});
