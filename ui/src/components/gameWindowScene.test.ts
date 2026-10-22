import assert from 'node:assert/strict';
import test from 'node:test';
import { MIN_ZOOM, described, fitZoom, keyedOffset, listAt, revealedOffset, scrolledOffset, wheelPixels } from './gameWindowScene.ts';

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
