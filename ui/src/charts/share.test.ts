import assert from 'node:assert/strict';
import test from 'node:test';
import { SHARE_TICKS, drawShareBands, shareLayer, sharePixels, shareSegments, shareStack } from './share.ts';
import { valueTicks } from './draw.ts';
import { plotFrame } from './frame.ts';
import { tokensFromValues } from './tokens.ts';

test('stacked shares retain internal changes, held values, identity breaks and missing hours', () => {
  const stack = shareStack([
    { firstHour: 1, lastHour: 2, counts: [1, 2, 1], startsSegment: true },
    { firstHour: 3, lastHour: 4, counts: [3, 0, 1] },
    { firstHour: 6, lastHour: 7, counts: [0, 0, 4], startsSegment: true },
    { firstHour: 8, lastHour: 9, counts: [0, 4, 0], startsSegment: true },
  ], 3);
  assert.deepEqual(stack.edges[0], [{ low: 0, high: 25 }, { low: 25, high: 75 }, { low: 75, high: 100 }]);
  const segments = shareSegments(stack, 1, 9);
  assert.equal(segments.length, 3);
  const at = segments[0].hours.indexOf(3);
  assert.equal(segments[0].bands[0].high[at], 25);
  assert.equal(segments[0].bands[0].high[at + 1], 75);
  assert.equal(stack.edges[stack.hours.indexOf(5)], null);
  assert.deepEqual(shareSegments(stack, 6, 5), []);
});

test('zero denominators are gaps and malformed counts are refused', () => {
  assert.deepEqual(shareStack([{ firstHour: 1, lastHour: 2, counts: [0, 0, 0] }], 3).hours, []);
  assert.throws(() => shareStack([{ firstHour: 1, lastHour: 2, counts: [0, 1] }], 3));
  assert.throws(() => shareStack([{ firstHour: 1, lastHour: 2, counts: [-1, 1, 2] }], 3));
});

test('pixel thinning retains internal share peaks, gaps, identities and exact model values', () => {
  const runs = Array.from({ length: 2_000 }, (_, hour) => ({ firstHour: hour, lastHour: hour,
    counts: hour === 71 ? [100, 0, 0] : [0, 0, 100], startsSegment: hour === 1_001 }));
  runs.splice(1_000, 1);
  const model = shareStack(runs, 3);
  const exact = shareSegments(model, 0, 1_999);
  const thinned = shareSegments(model, 0, 1_999, 20);
  assert.equal(thinned.length, exact.length);
  assert.ok(thinned.flatMap(segment => segment.hours).length < exact.flatMap(segment => segment.hours).length / 2);
  assert.ok(thinned.some(segment => segment.bands[0].high.includes(100)));
  assert.equal(model.edges[model.hours.indexOf(1_000)], null);
  assert.equal(model.edges[model.hours.indexOf(71)]![0].high, 100);
});

test('share axes keep quarters at every chart height and when zoomed', () => {
  const tokens = tokensFromValues(() => 'theme', 'font');
  for (const height of [150, 300, 800]) {
    const frame = plotFrame(400, height, { left: 20, right: 20, top: 20, bottom: 20 }, [1, 8], [0, 100], false);
    frame.valueTicks = SHARE_TICKS;
    assert.deepEqual(valueTicks(frame, tokens), SHARE_TICKS);
    frame.y = [20, 80];
    assert.deepEqual(valueTicks(frame, tokens), [25, 50, 75]);
  }
});

const bands = ['bad', 'warn', 'good'].map((color, index) => ({ id: color, label: color, color, alpha: index === 2 ? 0.85 : 1 }));
const twoRuns = () => shareStack([
  { firstHour: 0, lastHour: 2, counts: [1, 1, 8] },
  { firstHour: 6, lastHour: 9, counts: [2, 5, 3] },
], 3);

test('share drawing fills one held polygon per band per contiguous run, with no per-sample state or stroke', () => {
  const calls: string[] = [];
  const record = (key: string) => () => { calls.push(key); };
  const ctx = { save: record('save'), restore: record('restore'), beginPath: record('begin'),
    moveTo: record('move'), lineTo: record('line'), closePath: record('close'), fill: record('fill'),
    stroke: record('stroke'), set globalAlpha(_value: number) { calls.push('alpha'); } } as unknown as CanvasRenderingContext2D;
  const frame = plotFrame(16, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 16], [0, 100], false);
  drawShareBands(ctx, frame, shareSegments(twoRuns(), 0, 9, 1), bands);
  for (const key of ['begin', 'move', 'close', 'fill']) assert.equal(calls.filter(call => call === key).length, 6);
  assert.equal(calls.filter(call => call === 'alpha').length, 3);
  assert.equal(calls.filter(call => call === 'save').length, 1);
  assert.equal(calls.filter(call => call === 'restore').length, 1);
  assert.equal(calls.includes('stroke'), false);
});

test('sparse tile polygons stay within two vertices per device column per band plus exact run boundaries', () => {
  let points = 0; let fills = 0;
  const raster = { clearRect() {}, save() {}, restore() {}, beginPath() {},
    moveTo() { points++; }, lineTo() { points++; }, closePath() {}, fill() { fills++; },
    getImageData() { return {}; }, putImageData() {} };
  const model = shareStack(Array.from({ length: 15 }, (_, hour) => ({ firstHour: hour, lastHour: hour,
    counts: [hour % 3 + 1, 2, 7 - hour % 3] })), 3);
  const layer = shareLayer(model, bands, [0, 14], () => ({ getContext: () => raster }) as unknown as HTMLCanvasElement);
  const ctx = { getTransform: () => ({ a: 2, d: 2 }), save() {}, restore() {}, beginPath() {}, rect() {}, clip() {}, drawImage() {} } as unknown as CanvasRenderingContext2D;
  const frame = plotFrame(16, 8, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 16], [0, 100], false);
  layer.draw(ctx, frame, tokensFromValues(() => 'theme', 'font'), { playhead: null });
  assert.equal(fills, 3);
  assert.ok(points <= 3 * (2 * 32 + 4));
});

test('share tiles reuse sampling and fills through hover, playhead changes and moving domains; resize rebuilds', () => {
  const model = twoRuns();
  let reads = 0; let fills = 0; let images = 0; let materialized = 0;
  model.edges = new Proxy(model.edges, { get(target, key, receiver) {
    if (typeof key === 'string' && /^\d+$/.test(key)) reads++;
    return Reflect.get(target, key, receiver);
  } });
  const raster = { clearRect() {}, save() {}, restore() {}, beginPath() {}, moveTo() {}, lineTo() {}, closePath() {},
    fill() { fills++; }, getImageData() { return {}; }, putImageData() { materialized++; } };
  const createCanvas = () => ({ getContext: () => raster }) as unknown as HTMLCanvasElement;
  const layer = shareLayer(model, bands, [0, 9], createCanvas);
  const clips: number[][] = [];
  const ctx = { getTransform: () => ({ a: 1, d: 1 }), save() {}, restore() {}, beginPath() {},
    rect(...box: number[]) { clips.push(box); }, clip() {}, drawImage() { images++; } } as unknown as CanvasRenderingContext2D;
  const frame = plotFrame(16, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 16], [0, 100], false);
  const tokens = tokensFromValues(() => 'theme', 'font');
  layer.draw(ctx, frame, tokens, { playhead: 2 });
  const initialReads = reads;
  assert.ok(initialReads > 0);
  assert.equal(fills, 6);
  assert.equal(materialized, 1);
  layer.draw(ctx, frame, tokens, { playhead: 3 });
  layer.draw(ctx, frame, tokens, { playhead: 3 });
  layer.draw(ctx, plotFrame(16, 100, frame.padding, [1, 17], [0, 100], false), tokens, { playhead: 4 });
  assert.equal(reads, initialReads);
  assert.equal(fills, 6);
  assert.equal(materialized, 1);
  assert.equal(images, 4);
  assert.equal(clips[0][2], frame.toX(2) - frame.toX(0));
  layer.draw(ctx, plotFrame(32, 100, frame.padding, [0, 16], [0, 100], false), tokens, { playhead: 4 });
  assert.ok(reads > initialReads);
  assert.equal(fills, 12);
  assert.equal(materialized, 2);
});

const colors = new Uint8ClampedArray([255, 0, 0, 255, 255, 255, 0, 255, 0, 255, 0, 217]);
const pixels = { createImageData(width: number, height: number) {
  return { width, height, data: new Uint8ClampedArray(width * height * 4) };
} } as unknown as CanvasRenderingContext2D;

test('sparse path density uses the actual projected device width, not tile padding', () => {
  let points = 0; let fills = 0; let writes = 0;
  const raster = { clearRect() {}, save() {}, restore() {}, beginPath() {},
    moveTo() { points++; }, lineTo() { points++; }, closePath() {}, fill() { fills++; }, fillRect() {},
    createImageData: pixels.createImageData, getImageData() { return { data: colors }; }, putImageData() { writes++; } };
  const model = shareStack(Array.from({ length: 15 }, (_, hour) => ({ firstHour: hour, lastHour: hour,
    counts: [hour % 3 + 1, 2, 7 - hour % 3] })), 3);
  const layer = shareLayer(model, bands, [0, 14], () => ({ getContext: () => raster }) as unknown as HTMLCanvasElement);
  const ctx = { getTransform: () => ({ a: 2, d: 2 }), save() {}, restore() {}, beginPath() {}, rect() {}, clip() {}, drawImage() {} } as unknown as CanvasRenderingContext2D;
  const frame = plotFrame(16, 8, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 19], [0, 100], false);
  layer.draw(ctx, frame, tokensFromValues(() => 'theme', 'font'), { playhead: null });
  assert.equal(points, 0);
  assert.equal(fills, 0);
  assert.equal(writes, 1);
});

test('dense device columns retain each state contribution and the exact fractional gap coverage', () => {
  const frame = plotFrame(8, 8, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 16], [0, 100], false);
  const model = shareStack([
    { firstHour: 0, lastHour: 0, counts: [4, 0, 0] },
    { firstHour: 1, lastHour: 1, counts: [0, 0, 4] },
  ], 3);
  const mixed = sharePixels(pixels, model, frame, colors);
  for (let row = 0; row < 8; row++) {
    assert.deepEqual([...mixed.data.slice(row * 32, row * 32 + 4)], [138, 117, 0, 236]);
    assert.deepEqual([...mixed.data.slice(row * 32 + 4, (row + 1) * 32)], Array(28).fill(0));
  }
  const gaps = sharePixels(pixels, shareStack([
    { firstHour: 0, lastHour: 0, counts: [4, 0, 0] },
    { firstHour: 2, lastHour: 2, counts: [0, 0, 4] },
  ], 3), frame, colors);
  assert.deepEqual([...gaps.data.slice(0, 8)], [255, 0, 0, 128, 0, 255, 0, 108]);
  const zoomed = plotFrame(8, 8, frame.padding, frame.x, [25, 75], false);
  const middle = sharePixels(pixels, shareStack([{ firstHour: 0, lastHour: 0, counts: [1, 2, 1] }], 3), zoomed, colors);
  assert.deepEqual([...middle.data.slice(0, 4)], [255, 255, 0, 128]);
  assert.deepEqual([...middle.data.slice(-32, -28)], [255, 255, 0, 128]);
});

test('dense pixel preparation reads only the requested columns; zero-width bands never gain area', () => {
  const model = shareStack(Array.from({ length: 4_000 }, (_, hour) => ({ firstHour: hour, lastHour: hour, counts: [1, 0, 1] })), 3);
  const readHours: number[] = [];
  model.edges = new Proxy(model.edges, { get(target, key, receiver) {
    if (typeof key === 'string' && /^\d+$/.test(key)) readHours.push(model.hours[Number(key)]);
    return Reflect.get(target, key, receiver);
  } });
  const frame = plotFrame(80, 8, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 4_000], [0, 100], false);
  const image = sharePixels(pixels, model, frame, colors, 79);
  assert.equal(image.width, 1);
  assert.equal(readHours.length, 50);
  assert.ok(readHours.every(hour => hour >= 3_950));
  assert.deepEqual([...image.data.slice(0, 4)], [0, 255, 0, 217]);
  assert.deepEqual([...image.data.slice(-4)], [255, 0, 0, 255]);
});

test('pixel writing obtains native ImageData storage once per column, never per pixel', () => {
  let reads = 0;
  const ctx = { createImageData(width: number, height: number) {
    const data = new Uint8ClampedArray(width * height * 4);
    return { width, height, get data() { reads++; return data; } };
  } } as unknown as CanvasRenderingContext2D;
  const frame = plotFrame(8, 80, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 16], [0, 100], false);
  sharePixels(ctx, shareStack([{ firstHour: 0, lastHour: 15, counts: [1, 2, 1] }], 3), frame, colors);
  assert.equal(reads, 8);
});

test('appended data dirties only its frontier device column; playback never prepares previous pixels again', () => {
  const cleared: number[][] = []; const writes: number[][] = [];
  let reads = 0; let points = 0;
  const hoursRead: number[] = [];
  const raster = { save() {}, restore() {}, beginPath() {}, moveTo() { points++; }, lineTo() { points++; },
    closePath() {}, fill() {}, fillRect() {},
    clearRect(...box: number[]) { cleared.push(box); }, createImageData: pixels.createImageData,
    getImageData() { return { data: colors }; },
    putImageData(image: ImageData, x: number) { writes.push([image.width, x]); } };
  const createCanvas = () => ({ getContext: () => raster }) as unknown as HTMLCanvasElement;
  const history = (hours: number) => {
    const model = shareStack(Array.from({ length: hours }, (_, hour) => ({ firstHour: hour, lastHour: hour, counts: [hour % 5, 1, 9 - hour % 5] })), 3);
    model.edges = new Proxy(model.edges, { get(target, key, receiver) {
      if (typeof key === 'string' && /^\d+$/.test(key)) { reads++; hoursRead.push(model.hours[Number(key)]); }
      return Reflect.get(target, key, receiver);
    } });
    return model;
  };
  const model = history(60);
  const layer = shareLayer(model, bands, [0, 59], createCanvas);
  const ctx = { getTransform: () => ({ a: 1, d: 1 }), save() {}, restore() {}, beginPath() {}, rect() {}, clip() {}, drawImage() {} } as unknown as CanvasRenderingContext2D;
  const frame = plotFrame(8, 8, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 64], [0, 100], false);
  const tokens = tokensFromValues(() => 'theme', 'font');
  layer.draw(ctx, frame, tokens, { playhead: 30 });
  const preparedReads = reads;
  hoursRead.length = 0;
  layer.draw(ctx, frame, tokens, { playhead: 31 });
  layer.draw(ctx, frame, tokens, { playhead: 59 });
  assert.equal(reads - preparedReads, 10);
  assert.deepEqual(hoursRead, [24, 25, 26, 27, 28, 29, 30, 56, 57, 58]);
  assert.equal(writes.filter(([width]) => width === 8).length, 1, 'completed columns are never prepared by playback');
  assert.equal(points, 0, 'dense columns have no subpixel Canvas paths');
  assert.deepEqual(cleared, [[0, 0, 8, 8]]);
  layer.update(history(61), [0, 60]);
  reads = 0;
  layer.draw(ctx, frame, tokens, { playhead: 60 });
  assert.deepEqual(cleared.at(-1), [7, 0, 1, 8]);
  assert.deepEqual(writes.at(-2), [1, 7]);
  assert.equal(reads, 10, 'only the dirty frontier column and visible part of its edge are read');
});

test('share polygons clip fractional tile edges without snapping a narrow gap or spike', () => {
  const paths: number[][][] = [];
  let path: number[][];
  const ctx = { save() {}, restore() {}, beginPath() { path = []; paths.push(path); },
    moveTo(x: number, y: number) { path.push([x, y]); }, lineTo(x: number, y: number) { path.push([x, y]); },
    closePath() {}, fill() {} } as unknown as CanvasRenderingContext2D;
  const model = shareStack([
    { firstHour: 0, lastHour: 1, counts: [0, 0, 100] },
    { firstHour: 2, lastHour: 2, counts: [100, 0, 0] },
    { firstHour: 3, lastHour: 4, counts: [0, 0, 100] },
    { firstHour: 6, lastHour: 9, counts: [0, 0, 100] },
  ], 3);
  const frame = plotFrame(8, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [1.5, 6.5], [0, 100], false);
  drawShareBands(ctx, frame, shareSegments(model, 0, 9, 1), bands);
  assert.equal(paths.length, 3); // The spike and two separated in-supply stretches.
  const spike = paths[0];
  assert.ok(spike.some(([x, y]) => x === frame.toX(2) && y === frame.toY(100)));
  assert.ok(spike.some(([x, y]) => x === frame.toX(3) && y === frame.toY(100)));
  assert.ok(paths[1].every(([x]) => x >= 0 && x <= frame.toX(5)));
  assert.ok(paths[2].every(([x]) => x >= frame.toX(6) && x <= frame.plotWidth));
});
