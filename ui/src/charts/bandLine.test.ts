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

