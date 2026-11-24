import assert from 'node:assert/strict';
import test from 'node:test';
import { compactStack, compactStackExtent, forEachCompactStackSegment } from './stack.ts';
import { countryRaceStack, countryRaceStackExtent, forEachCountryRaceSegment } from './race/countryRace.ts';
import type { StackSegment } from './stack';

test('compact stacks retain signed bounds, missing bands, native breaks and held viewport edges', () => {
  const model = compactStack([
    [{ firstHour: 100, lastHour: 104, value: 2 }, { firstHour: 106, lastHour: 109, value: -3, startsSegment: true }],
    [{ firstHour: 100, lastHour: 109, value: 5 }],
  ]);
  assert.deepEqual(compactStackExtent(model, 102, 103), [0, 7]);
  assert.equal(compactStackExtent(model, 105, 105), null);
  assert.deepEqual(compactStackExtent(model, 106, 109), [-3, 5]);
  const pieces: StackSegment[] = [];
  forEachCompactStackSegment(model, 104.5, 108, 0, piece => pieces.push(piece));
  assert.deepEqual(pieces.map(piece => piece.hours), [[104.5, 105], [106, 108]]);
  assert.deepEqual(pieces[1].bands, [{ low: [-3, -3], high: [0, 0] }, { low: [0, 0], high: [5, 5] }]);
  const isolated: StackSegment[] = [];
  forEachCompactStackSegment(compactStack([[{ firstHour: 10, lastHour: 10, value: 1 }]]), 0, 30, 20, piece => isolated.push(piece));
  assert.deepEqual(isolated.map(piece => piece.hours), [[10, 11]]);
});

test('removing a wide band retains small signed bands in extents and streamed geometry', () => {
  for (const sign of [1, -1]) {
    const model = compactStack([
      [{ firstHour: 0, lastHour: 0, value: sign * Number.MAX_SAFE_INTEGER }, { firstHour: 1, lastHour: 2, value: 0 }],
      [{ firstHour: 0, lastHour: 2, value: sign * 2 }],
    ]);
    assert.deepEqual(compactStackExtent(model, 1, 2), sign === 1 ? [0, 2] : [-2, 0]);
    let shown: StackSegment | null = null;
    forEachCompactStackSegment(model, 1, 2, 0, piece => { shown = piece; });
    assert.ok(shown);
    assert.deepEqual((shown as StackSegment).bands[1], sign === 1 ? { low: [0, 0], high: [2, 2] } : { low: [-2, -2], high: [0, 0] });
  }
});


test('pixel thinning retains positive and negative aggregate extremes before the last point', () => {
  const bands = [[{ firstHour: 0, lastHour: 0, value: 1 }, { firstHour: 1, lastHour: 1, value: -50 },
    { firstHour: 2, lastHour: 2, value: 100 }, { firstHour: 3, lastHour: 10, value: 1 }]];
  const model = compactStack(bands);
  const pieces: StackSegment[] = [];
  forEachCompactStackSegment(model, 0, 10, 20, piece => pieces.push(piece));
  assert.deepEqual(compactStackExtent(model, 0, 10), [-50, 100]);
  assert.ok(pieces.some(piece => piece.hours.includes(1) && piece.bands[0].low.includes(-50)));
  assert.ok(pieces.some(piece => piece.hours.includes(2) && piece.bands[0].high.includes(100)));
});

test('optional geometry sampling uses clipped emitted hours and remains on the thinning grid', () => {
  const model = compactStack([Array.from({ length: 200 }, (_, hour) => ({ firstHour: hour, lastHour: hour, value: hour + 1 }))]);
  const sampled: number[] = [];
  const pieces: StackSegment[] = [];
  forEachCompactStackSegment(model, 2.5, 197.5, 20, piece => pieces.push(piece), hour => {
    sampled.push(hour);
    return [{ low: 0, high: hour + 0.25 }];
  });
  assert.equal(sampled[0], 2.5);
  assert.equal(sampled.at(-1), 197.5);
  assert.ok(sampled.length <= Math.ceil((197.5 - 2.5) / 20) + 3);
  assert.ok(sampled.length < model.hours.length);
  assert.deepEqual([...new Set(pieces.flatMap(piece => piece.hours))], sampled);
  for (const piece of pieces) assert.deepEqual(piece.bands[0].high, piece.hours.map(hour => hour + 0.25));
  assert.equal(pieces[0].start, true);
  assert.equal(pieces.at(-1)!.end, true);
});

test('sampled clipped starts compare their drawn bounds when retaining an interior extreme', () => {
  for (const sign of [1, -1]) {
    const model = compactStack([[{ firstHour: 0, lastHour: 0, value: sign * 100 }, { firstHour: 1, lastHour: 1, value: sign * 50 },
      { firstHour: 2, lastHour: 3, value: sign }]]);
    const pieces: StackSegment[] = [];
    forEachCompactStackSegment(model, 0.5, 2.5, 10, piece => pieces.push(piece), hour => {
      const value = sign * (hour === 1 ? 50 : 1);
      return [{ low: Math.min(0, value), high: Math.max(0, value) }];
    });
    assert.ok(pieces.some(piece => piece.hours.includes(1) && (sign === 1 ? piece.bands[0].high : piece.bands[0].low).includes(sign * 50)));
    assert.deepEqual(pieces[0].hours, [0.5, 1]);
  }
});

test('the CountryRaceView boundary streams 5000 constant parts plus a daily-changing part on the drawing grid', () => {
  const count = 5000; const days = 3650; const lastHour = days * 24 - 1;
  const bands = Array.from({ length: count }, () => [{ firstHour: 0, lastHour, value: 1 }]);
  bands.push(Array.from({ length: days }, (_, day) => ({ firstHour: day * 24, lastHour: day * 24 + 23, value: day + 1 })));
  const model = countryRaceStack(bands, true);
  assert.ok('bands' in model);
  assert.equal('edges' in model, false);
  assert.equal(model.bands, bands);
  assert.ok(model.hours.length <= days * 2 + 1);
  assert.deepEqual(countryRaceStackExtent(model, 0, lastHour), [0, count + days]);
  const pixels = 10;
  let drawPieces = 0; let drawnBandPoints = 0;
  forEachCountryRaceSegment(model, 0, lastHour, lastHour / pixels, true, piece => {
    drawPieces++;
    assert.equal(piece.bands.length, count + 1);
    assert.ok(piece.hours.length <= 3);
    drawnBandPoints += piece.hours.length * piece.bands.length;
  });
  assert.ok(drawPieces <= pixels * 6 + 2);
  assert.ok(drawnBandPoints <= (pixels * 6 + 2) * (count + 1) * 3);
});
