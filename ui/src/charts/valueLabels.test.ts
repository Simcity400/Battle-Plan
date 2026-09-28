import assert from 'node:assert/strict';
import test from 'node:test';
import { VALUE_LABEL_GAP, valueLabelWidth, valueTicks } from './draw.ts';
import { plotFrame } from './frame.ts';
import { tokensFromValues } from './tokens.ts';

/** A canvas whose text is 7 pixels a character, recording what it measured. */
function measuring() {
  const measured: string[] = [];
  const context = { font: '', save() {}, restore() {}, measureText(text: string) { measured.push(text); return { width: text.length * 7 }; } };
  return { context: context as unknown as CanvasRenderingContext2D, measured };
}

const tokens = tokensFromValues(() => 'rgb(0, 0, 0)', 'sans-serif');

test('the widest value label is measured from the ticks the axis draws, with every tick given', () => {
  const frame = plotFrame(600, 300, { left: 44, right: 14, top: 12, bottom: 24 }, [0, 100], [100000, 100000.4], false);
  const ticks = valueTicks(frame, tokens);
  const { context, measured } = measuring();
  const seen: (readonly number[])[] = [];
  const format = (value: number, all: readonly number[]) => { seen.push(all); return value.toLocaleString('en-US', { minimumFractionDigits: 1 }); };
  const width = valueLabelWidth(context, frame, tokens, format);
  // "100,000.4" is nine characters: 63 pixels, more than the 44 a chart keeps by default.
  assert.equal(width, 63);
  assert.ok(Math.ceil(width) + VALUE_LABEL_GAP > 44);
  assert.equal(measured.length, ticks.length);
  assert.ok(seen.every(all => all.length === ticks.length));
});

test('ticks do not depend on the left padding, so widening it keeps the same labels', () => {
  const narrow = plotFrame(600, 300, { left: 44, right: 14, top: 12, bottom: 24 }, [0, 100], [0, 12345], false);
  const wide = plotFrame(600, 300, { left: 90, right: 14, top: 12, bottom: 24 }, [0, 100], [0, 12345], false);
  assert.deepEqual(valueTicks(narrow, tokens), valueTicks(wide, tokens));
});
