import assert from 'node:assert/strict';
import test from 'node:test';
import { drawBandLine, drawBands } from './draw.ts';
import { plotFrame } from './frame.ts';
import { buildStack, stackedSegments } from './stack.ts';
import { tokensFromValues } from './tokens.ts';

function recorder() {
  type Vertex = ['M' | 'L', number, number];
  const paths: Vertex[][] = [];
  const rects: number[][] = [];
  let current: Vertex[] = [];
  const context = {
    save() {}, restore() {}, stroke() {}, fill() {}, closePath() {}, setLineDash() {},
    beginPath() { current = []; paths.push(current); },
    moveTo(x: number, y: number) { current.push(['M', x, y]); },
    lineTo(x: number, y: number) { current.push(['L', x, y]); },
    fillRect(...values: number[]) { rects.push(values); },
  };
  return { ctx: context as unknown as CanvasRenderingContext2D, paths, rects };
}

const frame = plotFrame(123, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [100, 223], [0, 100], false);
const tokens = tokensFromValues(() => 'rgb(0, 0, 0)', 'sans-serif');
const stack = buildStack([[
  { firstHour: 100, lastHour: 123, value: 50 },
  { firstHour: 124, lastHour: 147, value: 100 },
  { firstHour: 200, lastHour: 223, value: 30, startsSegment: true },
]]);

test('a band line joins precisely the band edge vertices, keeping 50% at hour 118', () => {
  const segments = stackedSegments(stack, 100, 223, 0.5);
  const bands = recorder();
  const line = recorder();
  for (const segment of segments) {
    drawBands(bands.ctx, frame, tokens, segment.hours, segment.bands, ['country']);
    drawBandLine(line.ctx, frame, tokens, segment.hours, segment.bands[0].high, 'total');
  }
  assert.equal(line.paths.length, 2);
  segments.forEach((segment, index) => assert.deepEqual(line.paths[index], bands.paths[index].slice(0, segment.hours.length)));
  assert.deepEqual(line.paths[0].slice(0, 2), [['M', frame.toX(100), frame.toY(50)], ['L', frame.toX(123), frame.toY(50)]]);
  // The entire edge between these vertices, including hour 118, stays at 50%.
  assert.equal(line.paths[0][0][2], line.paths[0][1][2]);
  assert.ok(line.paths[0].at(-1)![1] < frame.toX(200));
});

test('a band line shares clipped and merged samples and never bridges a gap', () => {
  for (const [from, to, minHours] of [[110, 140, 0.5], [100, 223, 30], [170, 190, 0.5], [140, 120, 0.5]]) {
    const segments = stackedSegments(stack, from, to, minHours);
    const bands = recorder();
    const line = recorder();
    for (const segment of segments) {
      drawBands(bands.ctx, frame, tokens, segment.hours, segment.bands, ['country']);
      drawBandLine(line.ctx, frame, tokens, segment.hours, segment.bands[0].high, 'total');
    }
    assert.equal(line.paths.length, segments.length);
    segments.forEach((segment, index) => assert.deepEqual(line.paths[index], bands.paths[index].slice(0, segment.hours.length)));
  }
});

test('a single band sample has the same two-pixel top edge as its line', () => {
  const bands = recorder();
  const line = recorder();
  drawBands(bands.ctx, frame, tokens, [200], [{ low: [0], high: [30] }], ['country']);
  drawBandLine(line.ctx, frame, tokens, [200], [30], 'total');
  const [x, y, width] = bands.rects[0];
  assert.deepEqual(line.paths, [[['M', x, y], ['L', x + width, y]]]);
  drawBandLine(line.ctx, frame, tokens, [], [], 'total');
  assert.equal(line.paths.length, 1);
});
