import assert from 'node:assert/strict';
import test from 'node:test';
import { drawGapFill, drawLine } from './draw.ts';
import { plotFrame } from './frame.ts';
import { curveSegments, gapRegions } from './series.ts';
import type { Run } from './series.ts';
import { tokensFromValues } from './tokens.ts';

function recorder() {
  const paths: { calls: string[]; clipped: boolean; filled: string | null }[] = [];
  let current: { calls: string[]; clipped: boolean; filled: string | null } | null = null;
  const context = {
    fillStyle: '', strokeStyle: '', globalAlpha: 1, lineWidth: 1, lineJoin: '', lineCap: '',
    save() {}, restore() {}, setLineDash() {}, stroke() {}, arc() {},
    beginPath() { current = { calls: [], clipped: false, filled: null }; paths.push(current); },
    closePath() { current!.calls.push('Z'); },
    moveTo(x: number, y: number) { current!.calls.push(`M${x.toFixed(3)},${y.toFixed(3)}`); },
    lineTo(x: number, y: number) { current!.calls.push(`L${x.toFixed(3)},${y.toFixed(3)}`); },
    bezierCurveTo(...values: number[]) { current!.calls.push(`C${values.map(value => value.toFixed(3)).join(',')}`); },
    clip() { current!.clipped = true; },
    fill() { current!.filled = String(context.fillStyle); },
  };
  return { context: context as unknown as CanvasRenderingContext2D, paths };
}

const SUPPLY: Run[] = [{ firstHour: 0, lastHour: 40, value: 300, breakBefore: true }, { firstHour: 41, lastHour: 100, value: 100 }];
const DEMAND: Run[] = [{ firstHour: 0, lastHour: 40, value: 150, breakBefore: true }, { firstHour: 41, lastHour: 100, value: 200 }];
const frame = plotFrame(600, 300, { left: 40, right: 10, top: 10, bottom: 20 }, [0, 100], [0, 400], false);
const tokens = tokensFromValues(() => 'rgb(0, 0, 0)', 'sans-serif');

test('gap regions are bounded by the very curves the lines are drawn with', () => {
  const supply = curveSegments(SUPPLY, 0, 100, frame.toX, frame.toY);
  const demand = curveSegments(DEMAND, 0, 100, frame.toX, frame.toY);
  const { above, below } = gapRegions(supply, demand, frame.plotTop, frame.plotTop + frame.plotHeight);
  assert.equal(above.length, 1);
  assert.deepEqual(above[0].clip.slice(0, supply[0].path.length), supply[0].path);
  assert.deepEqual(above[0].fill.slice(0, demand[0].path.length), demand[0].path);
  assert.deepEqual(below[0].clip.slice(0, demand[0].path.length), demand[0].path);
  assert.deepEqual(below[0].fill.slice(0, supply[0].path.length), supply[0].path);
  const end = supply[0].points.at(-1)!;
  assert.deepEqual(above[0].clip.slice(-3), [['L', end.x, frame.plotTop + frame.plotHeight], ['L', supply[0].points[0].x, frame.plotTop + frame.plotHeight],
    ['L', supply[0].points[0].x, supply[0].points[0].y]]);
  assert.deepEqual(above[0].fill.at(-3), ['L', demand[0].points.at(-1)!.x, frame.plotTop]);
});

test('the fill a canvas draws follows the drawn lines, green above and red below', () => {
  const lines = recorder();
  drawLine(lines.context, frame, tokens, SUPPLY, 'supply', { endDot: false });
  drawLine(lines.context, frame, tokens, DEMAND, 'demand', { endDot: false, dash: [6, 4] });
  const curves = lines.paths.map(path => path.calls.filter(call => call.startsWith('M') || call.startsWith('C')));
  const fill = recorder();
  drawGapFill(fill.context, frame, SUPPLY, DEMAND, 'green', 'red');
  const [surplusClip, surplusFill, shortfallClip, shortfallFill] = fill.paths;
  const curveOf = (calls: string[]) => calls.filter(call => call.startsWith('M') || call.startsWith('C'));
  assert.deepEqual(curveOf(surplusClip.calls), curves[0]);
  assert.equal(surplusClip.clipped, true);
  assert.deepEqual(curveOf(surplusFill.calls), curves[1]);
  assert.equal(surplusFill.filled, 'green');
  assert.deepEqual(curveOf(shortfallClip.calls), curves[1]);
  assert.deepEqual(curveOf(shortfallFill.calls), curves[0]);
  assert.equal(shortfallFill.filled, 'red');
  const until = recorder();
  drawGapFill(until.context, frame, SUPPLY, DEMAND, 'green', 'red', { until: 30 });
  const partial = recorder();
  drawLine(partial.context, frame, tokens, SUPPLY, 'supply', { endDot: false, until: 30 });
  assert.deepEqual(curveOf(until.paths[0].calls), curveOf(partial.paths[0].calls));
});

test('a gap in the recording leaves a gap in the fill', () => {
  const broken: Run[] = [{ firstHour: 0, lastHour: 20, value: 300 }, { firstHour: 30, lastHour: 60, value: 280 }];
  const lower: Run[] = [{ firstHour: 0, lastHour: 20, value: 100 }, { firstHour: 30, lastHour: 60, value: 120 }];
  const { above } = gapRegions(curveSegments(broken, 0, 100, frame.toX, frame.toY), curveSegments(lower, 0, 100, frame.toX, frame.toY), 0, 300);
  assert.equal(above.length, 2);
});
