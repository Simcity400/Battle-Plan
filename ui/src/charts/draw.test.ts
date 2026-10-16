import assert from 'node:assert/strict';
import test from 'node:test';
import { drawLine } from './draw.ts';
import { plotFrame } from './frame.ts';
import { heldSegments } from './series.ts';
import { tokensFromValues } from './tokens.ts';

test('shared held lines preserve exact reads, gaps, identity changes and fractional zoom edges', () => {
  const paths = heldSegments([
    { firstHour: 1, lastHour: 2, value: 2 }, { firstHour: 3, lastHour: 4, value: 8 },
    { firstHour: 6, lastHour: 6, value: 9 }, { firstHour: 7, lastHour: 8, value: 9, breakBefore: true },
  ], 1, 8, value => value, value => value).map(segment => segment.path);
  assert.deepEqual(paths, [
    [['M', 1, 2], ['L', 3, 2], ['L', 3, 8], ['L', 5, 8]],
    [['M', 6, 9], ['L', 7, 9]], [['M', 7, 9], ['L', 8, 9]],
  ]);
  assert.deepEqual(heldSegments([{ firstHour: 1, lastHour: 4, value: 2 }], 2.5, 3.5, value => value, value => value)[0].path,
    [['M', 2.5, 2], ['L', 3.5, 2]]);
  assert.deepEqual(heldSegments([{ firstHour: 1, lastHour: 4, value: 2 }], 5, 6, value => value, value => value), []);
  assert.deepEqual(heldSegments([{ firstHour: 1, lastHour: 4, value: 2 }], 3, 2, value => value, value => value), []);
});

test('drawLine held option shares its end dot and theme widths and never smooths a read', () => {
  const commands: { kind: string; values: number[] }[] = [];
  const record = (kind: string) => (...values: number[]) => { commands.push({ kind, values }); };
  const ctx = { beginPath: record('begin'), moveTo: record('M'), lineTo: record('L'), bezierCurveTo: record('C'),
    arc: record('arc'), stroke: record('stroke'), fill: record('fill'), setLineDash: () => {} } as unknown as CanvasRenderingContext2D;
  const frame = plotFrame(400, 200, { left: 20, right: 20, top: 20, bottom: 20 }, [1, 8], [0, 10], false);
  const tokens = { ...tokensFromValues(() => 'transparent', 'sans-serif'), lineWidth: { grid: 1, series: 3, emphasis: 4 } };
  const end = drawLine(ctx, frame, tokens, [{ firstHour: 1, lastHour: 2, value: 2 }, { firstHour: 3, lastHour: 8, value: 8 }],
    'country-colour', { held: true, until: 5 });
  assert.deepEqual(end, { x: frame.toX(5), y: frame.toY(8) });
  assert.equal(commands.some(command => command.kind === 'C'), false);
  assert.deepEqual(commands.filter(command => command.kind === 'L').map(command => command.values),
    [[frame.toX(3), frame.toY(2)], [frame.toX(3), frame.toY(8)], [frame.toX(5), frame.toY(8)]]);
  assert.deepEqual(commands.find(command => command.kind === 'arc')?.values.slice(0, 3), [end!.x, end!.y, tokens.lineWidth.series * 2]);
  assert.equal(ctx.lineWidth, tokens.lineWidth.series);
  assert.equal(ctx.strokeStyle, tokens.surface);
});

test('line geometry follows the complete time scale while its drawn end stays fixed', () => {
  const ctx = { beginPath() {}, moveTo() {}, lineTo() {}, bezierCurveTo() {}, arc() {}, stroke() {}, fill() {}, setLineDash() {} } as unknown as CanvasRenderingContext2D;
  const tokens = tokensFromValues(() => 'transparent', 'sans-serif');
  const runs = [{ firstHour: 0, lastHour: 100, value: 1 }];
  for (const held of [false, true]) {
    for (const end of [100, 200, 50]) {
      const frame = plotFrame(400, 200, { left: 20, right: 20, top: 20, bottom: 20 }, [0, end], [0, 2], false);
      assert.deepEqual(drawLine(ctx, frame, tokens, runs, 'colour', { held, until: 40 }), { x: frame.toX(40), y: frame.toY(1) });
    }
  }
});
