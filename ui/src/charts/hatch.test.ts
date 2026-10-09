import { fillHatched as raceHatch } from './race/draw.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import { fillHatched, hatchStyle, hatchTile } from './hatch.ts';
import { tokensFromValues } from './tokens.ts';
import { drawRecordedGap } from './draw.ts';
import { plotFrame } from './frame.ts';

test('unknown hatching uses the same token-sized tile in Canvas and HTML', () => {
  assert.equal(raceHatch, fillHatched);
  const tokens = { ...tokensFromValues(() => '#8899aa', 'sans-serif'), hatchSpacing: 12 };
  const tile = hatchTile(tokens.hatchSpacing), style = hatchStyle(tokens);
  assert.deepEqual(tile, { size: 12, lines: [[0, 12, 12, 0]] });
  assert.equal(style.backgroundSize, '12px 12px');
  const svg = decodeURIComponent(style.backgroundImage);
  assert.ok(svg.includes('width="12"'));
  assert.ok(svg.includes('M0,12L12,0'));
  assert.ok(svg.includes('opacity="0.25"'));
  assert.ok(svg.includes('stroke="#8899aa"'));
});

test('recording gap shading clips to the time view and the Live playhead', () => {
  const fills: number[][] = [];
  const ctx = { save() {}, restore() {}, fillRect(...rect: number[]) { fills.push(rect); } } as unknown as CanvasRenderingContext2D;
  const frame = plotFrame(100, 100, { left: 0, right: 0, top: 10, bottom: 10 }, [10, 110], [0, 1], false);
  const tokens = tokensFromValues(() => '#8899aa', 'sans-serif');
  drawRecordedGap(ctx, frame, tokens, 0, 23, 20);
  drawRecordedGap(ctx, frame, tokens, 72, 95, 20);
  assert.deepEqual(fills, [[0, 10, 11, 80]]);
});


test('shared hatch defaults preserve the race spacing and stroke', () => {
  const starts: number[] = [];
  const ctx = { globalAlpha: 1, lineWidth: 0, save() {}, restore() {}, beginPath() {}, rect() {}, clip() {}, fillRect() {},
    moveTo(x: number) { starts.push(x); }, lineTo() {}, stroke() {} } as unknown as CanvasRenderingContext2D;
  fillHatched(ctx, 0, 0, 12, 4, '#8899aa');
  assert.deepEqual(starts, [-4, 0, 4, 8]);
  assert.equal(ctx.lineWidth, 1.5);
});
