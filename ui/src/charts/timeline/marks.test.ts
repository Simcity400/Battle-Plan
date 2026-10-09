import assert from 'node:assert/strict';
import test from 'node:test';
import { acceptsLaneClick, acceptsLaneMenu, drawCross, drawDisclosure } from './marks.ts';

test('the first click expands once; repeated double-click clicks and drags do not toggle', () => {
  assert.equal(acceptsLaneClick(1, false), true);
  assert.equal(acceptsLaneClick(0, false), true);
  for (const detail of [2, 3]) assert.equal(acceptsLaneClick(detail, false), false);
  assert.equal(acceptsLaneClick(1, true), false);
});

test('Jump to ignores names unless the caller explicitly offers label actions', () => {
  assert.equal(acceptsLaneMenu(119, 120), false);
  assert.equal(acceptsLaneMenu(120, 120), true);
  assert.equal(acceptsLaneMenu(800, 120), true);
  assert.equal(acceptsLaneMenu(20, 120, true), true);
});

test('capitulation crosses use two eight-pixel diagonals in the supplied text colour', () => {
  const lines: number[][] = [];
  let strokes = 0;
  const context = { strokeStyle: '', lineWidth: 0, beginPath() {},
    moveTo(x: number, y: number) { lines.push([x, y]); },
    lineTo(x: number, y: number) { lines.push([x, y]); }, stroke() { strokes++; } };
  drawCross(context, 100, 40, 'theme text');
  assert.deepEqual(lines, [[96, 36], [104, 44], [104, 36], [96, 44]]);
  assert.equal(context.strokeStyle, 'theme text');
  assert.equal(context.lineWidth, 2);
  assert.equal(strokes, 1);
});

test('expanded and collapsed disclosure marks take the text token', () => {
  for (const expanded of [false, true]) {
    const points: number[][] = [];
    let fills = 0;
    const context = { fillStyle: '', beginPath() {}, closePath() {},
      moveTo(x: number, y: number) { points.push([x, y]); },
      lineTo(x: number, y: number) { points.push([x, y]); }, fill() { fills++; } };
    drawDisclosure(context, 12, 40, expanded, { text: 'theme text' });
    assert.equal(context.fillStyle, 'theme text');
    assert.equal(fills, 1);
    assert.deepEqual(points, expanded ? [[8, 38], [16, 38], [12, 43]] : [[10, 36], [15, 40], [10, 44]]);
  }
});
