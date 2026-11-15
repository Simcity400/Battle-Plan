import test from 'node:test';
import assert from 'node:assert/strict';
import { APPROXIMATE_ALPHA, APPROXIMATE_DASH, PENDING_ALPHA, markApproximate, overlapsSpans, pendingLayer, preparedKeyItems, spanRanges } from './prepared.ts';
import { plotFrame } from './frame.ts';
import { tokensFromValues } from './tokens.ts';

/** The drawing calls a painter makes, with the style each was made in. */
class Recorder {
  globalAlpha = 1; fillStyle: unknown = ''; strokeStyle: unknown = ''; lineWidth = 0;
  calls: unknown[][] = [];
  save() {}
  restore() { this.globalAlpha = 1; }
  setLineDash(dash: number[]) { this.calls.push(['dash', dash]); }
  fillRect(...rect: number[]) { this.calls.push(['fill', this.fillStyle, this.globalAlpha, ...rect]); }
  strokeRect(...rect: number[]) { this.calls.push(['stroke', this.strokeStyle, ...rect]); }
  get context() { return this as unknown as CanvasRenderingContext2D; }
}

const tokens = tokensFromValues(variable => variable === '--surface' ? '#101010' : variable === '--muted' ? '#8899aa' : '#ffffff', 'sans-serif');

test('marked spans clip to the half-open view and stop after the playhead hour', () => {
  const spans = [{ firstHour: 0, lastHour: 9 }, { firstHour: 20, lastHour: 29 }, { firstHour: 40, lastHour: 49 }];
  assert.deepEqual(spanRanges(spans, 5, 45), [[5, 10], [20, 30], [40, 45]]);
  assert.deepEqual(spanRanges(spans, 5, 45, 24), [[5, 10], [20, 25]]);
  assert.deepEqual(spanRanges(spans, 10, 20), []);
  assert.deepEqual(spanRanges(undefined, 0, 100), []);
  assert.equal(overlapsSpans(spans, 10, 19), false);
  assert.equal(overlapsSpans(spans, 10, 20), true);
  assert.equal(overlapsSpans(undefined, 0, 100), false);
});

test('one key line names approximate spans and hours still being prepared, only when drawn', () => {
  const span = [{ firstHour: 1, lastHour: 2 }];
  assert.deepEqual(preparedKeyItems(undefined, undefined), []);
  assert.deepEqual(preparedKeyItems([], []), []);
  assert.deepEqual(preparedKeyItems(span, undefined), ['approximate']);
  assert.deepEqual(preparedKeyItems(undefined, span), ['pending']);
  assert.deepEqual(preparedKeyItems(span, span), ['approximate', 'pending']);
  assert.deepEqual(preparedKeyItems(undefined, [], span), ['loading']);
  assert.deepEqual(preparedKeyItems(span, span, span), ['approximate', 'pending', 'loading']);
});

test('pending hours draw a light band through the playhead and no geometry', () => {
  const recorder = new Recorder();
  const frame = plotFrame(100, 100, { left: 0, right: 0, top: 10, bottom: 10 }, [0, 100], [0, 1], false);
  const layer = pendingLayer([{ firstHour: 50, lastHour: 99 }]);
  layer.draw(recorder.context, frame, tokens, { playhead: null });
  layer.draw(recorder.context, frame, tokens, { playhead: 59 });
  layer.draw(recorder.context, frame, tokens, { playhead: 40 });
  assert.deepEqual(recorder.calls, [['fill', tokens.muted, PENDING_ALPHA, 50, 10, 50, 80], ['fill', tokens.muted, PENDING_ALPHA, 50, 10, 10, 80]]);
  const empty = new Recorder();
  pendingLayer(undefined).draw(empty.context, frame, tokens, { playhead: null });
  assert.equal(empty.calls.length, 0);
});

test('approximate geometry fades through the surface and takes the dashed edge of approximate lines', () => {
  const recorder = new Recorder();
  markApproximate(recorder.context, tokens, 10, 20, 30, 8);
  assert.deepEqual(recorder.calls, [['fill', tokens.surface, 1 - APPROXIMATE_ALPHA, 10, 20, 30, 8], ['dash', [...APPROXIMATE_DASH]],
    ['stroke', tokens.muted, 10.5, 20.5, 29, 7]]);
  const empty = new Recorder();
  markApproximate(empty.context, tokens, 10, 20, 0, 8);
  assert.equal(empty.calls.length, 0);
});
