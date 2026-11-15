/** Exact held stacked shares, built from counts over one common denominator.
 * A zero denominator and every missing band leave a gap, never a full share. */
import { buildStack, stackedSegments } from './stack.ts';
import type { StackModel, StackRun, StackSegment } from './stack';
import { heldStackSegment } from './race/countryRace.ts';
import type { Breakable } from './series';
import type { PlotFrame } from './frame';
import { plotFrame } from './frame.ts';
import type { ChartLayer } from './TimeChart';
import type { ChartSummary } from './chartSummary';

export type ShareRun = Breakable & { counts: readonly number[] };
export const SHARE_TICKS = [0, 25, 50, 75, 100] as const;
export interface ShareBand { id: string; label: string; color: string; alpha?: number }

/** Dense display samples use complete bucket coverage; broken buckets keep gaps. */
export function summaryShareRuns(runs: readonly ShareRun[], summaries?: readonly ChartSummary[]): readonly ShareRun[] {
  if (!summaries?.length || summaries[0].runs !== undefined) return runs;
  const result: ShareRun[] = [];
  let start = 0;
  for (const bucket of summaries[0].buckets) {
    if (!bucket) continue;
    while (start < runs.length && runs[start].lastHour < bucket.firstHour) start++;
    for (let index = start; index < runs.length && runs[index].firstHour <= bucket.lastHour; index++) {
      const run = runs[index], next = runs[index + 1];
      const firstHour = Math.max(run.firstHour, bucket.firstHour);
      const lastHour = bucket.hasBreak ? Math.min(run.lastHour, bucket.lastHour)
        : Math.min(bucket.lastHour, next ? next.firstHour - 1 : bucket.lastHour);
      if (firstHour > lastHour) continue;
      const previous = result.at(-1);
      if (previous && previous.lastHour + 1 === firstHour && !run.startsSegment && previous.counts.every((count, index) => count === run.counts[index])) previous.lastHour = lastHour;
      else result.push({ ...run, firstHour, lastHour, startsSegment: run.startsSegment && firstHour === run.firstHour, breakBefore: run.breakBefore && firstHour === run.firstHour });
    }
  }
  return result;
}

export function shareStack(runs: readonly ShareRun[], bands: number): StackModel {
  const values: StackRun[][] = Array.from({ length: bands }, () => []);
  for (const run of runs) {
    const total = run.counts.reduce((sum, count) => sum + count, 0);
    if (run.counts.length !== bands || !run.counts.every(count => Number.isSafeInteger(count) && count >= 0)) {
      throw new Error('Stacked share counts must be complete nonnegative integers.');
    }
    if (!total) continue;
    run.counts.forEach((count, index) => values[index].push({ firstHour: run.firstHour, lastHour: run.lastHour,
      breakBefore: run.breakBefore, startsSegment: run.startsSegment, value: count / total * 100 }));
  }
  return buildStack(values);
}

/** Half-pixel sampling retains internal band extremes, gaps and exact hover data. */
export function shareSegments(model: StackModel, from: number, to: number, hoursPerPixel = 0) {
  return stackedSegments(model, from, to, hoursPerPixel / 2, true).map(segment => {
    const last = segment.hours.at(-1)!;
    const index = heldIndex(model.hours, last);
    const end = Math.min(to, model.hours[index + 1] ?? to);
    if (model.edges[index] && end > last) {
      segment.hours.push(end);
      for (const band of segment.bands) { band.low.push(band.low.at(-1)!); band.high.push(band.high.at(-1)!); }
    }
    return heldStackSegment(segment);
  });
}

function heldIndex(hours: readonly number[], hour: number): number {
  let low = 0; let high = hours.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (hours[middle] <= hour) low = middle + 1; else high = middle;
  }
  return Math.max(0, low - 1);
}

/** One unsnapped, held polygon and fill per band per recorded stretch. */
export function drawShareBands(ctx: CanvasRenderingContext2D, frame: PlotFrame,
  segments: readonly StackSegment[], bands: readonly ShareBand[]): void {
  ctx.save();
  bands.forEach((look, band) => {
    ctx.fillStyle = look.color;
    ctx.globalAlpha = look.alpha ?? 1;
    for (const segment of segments) {
      const edges = segment.bands[band];
      const from = Math.max(segment.hours[0], frame.x[0]);
      const to = Math.min(segment.hours.at(-1)!, frame.x[1]);
      if (!(to > from)) continue;
      const first = heldIndex(segment.hours, from); const last = heldIndex(segment.hours, to);
      let visible = false;
      for (let index = first; index <= last; index++) visible ||= edges.high[index] !== edges.low[index];
      if (!visible) continue;
      ctx.beginPath();
      ctx.moveTo(frame.toX(from), frame.toY(edges.high[first]));
      for (let index = first + 1; index <= last; index++) {
        ctx.lineTo(frame.toX(segment.hours[index]), frame.toY(edges.high[index]));
      }
      ctx.lineTo(frame.toX(to), frame.toY(edges.high[last]));
      ctx.lineTo(frame.toX(to), frame.toY(edges.low[last]));
      for (let index = last; index > first; index--) {
        ctx.lineTo(frame.toX(segment.hours[index]), frame.toY(edges.low[index]));
      }
      ctx.lineTo(frame.toX(from), frame.toY(edges.low[first]));
      ctx.closePath();
      ctx.fill();
    }
  });
  ctx.restore();
}

/** Integrate held areas in device columns. Signed row differences keep the work
 * proportional to input edges plus output pixels, without subpixel Canvas paths.
 * A partial missing hour contributes no pixels, rather than a zero-valued share. */
export function sharePixels(ctx: CanvasRenderingContext2D, model: StackModel, frame: PlotFrame,
  colors: Uint8ClampedArray, firstColumn = 0, lastColumn = frame.width): ImageData {
  const width = frame.width; const height = frame.height;
  const image = ctx.createImageData(lastColumn - firstColumn, height);
  const changes = new Float64Array((height + 1) * 4);
  const premultiplied = Float64Array.from(colors, (value, index) => index % 4 === 3 ? value : value * colors[index - index % 4 + 3] / 255);
  const boundaries = Float64Array.from(premultiplied, (value, index) => value - (premultiplied[index + 4] ?? 0));
  const step = (frame.x[1] - frame.x[0]) / width;
  let index = heldIndex(model.hours, frame.x[0] + firstColumn * step);
  for (let column = firstColumn; column < lastColumn; column++) {
    index = shareColumn(model, frame, premultiplied, boundaries, changes, image, column - firstColumn,
      frame.x[0] + column * step, frame.x[0] + (column + 1) * step, index);
  }
  return image;
}

function addBoundary(changes: Float64Array, colors: Float64Array, y: number, weight: number, band: number): void {
  const first = Math.floor(y); const fraction = y - first;
  const offset = band * 4; const a = first * 4; const b = a + 4;
  const red = colors[offset] * weight; const green = colors[offset + 1] * weight;
  const blue = colors[offset + 2] * weight; const alpha = colors[offset + 3] * weight;
  changes[a] += red * (1 - fraction); changes[b] += red * fraction;
  changes[a + 1] += green * (1 - fraction); changes[b + 1] += green * fraction;
  changes[a + 2] += blue * (1 - fraction); changes[b + 2] += blue * fraction;
  changes[a + 3] += alpha * (1 - fraction); changes[b + 3] += alpha * fraction;
}

function writeColumn(changes: Float64Array, image: ImageData, column: number): void {
  const data = image.data; const stride = image.width * 4; const height = image.height;
  let red = 0; let green = 0; let blue = 0; let alpha = 0;
  let offset = column * 4;
  for (let row = 0; row < height; row++, offset += stride) {
    red += changes[row * 4]; green += changes[row * 4 + 1];
    blue += changes[row * 4 + 2]; alpha += changes[row * 4 + 3];
    if (Math.round(alpha) > 0) {
      data[offset] = red * 255 / alpha; data[offset + 1] = green * 255 / alpha;
      data[offset + 2] = blue * 255 / alpha; data[offset + 3] = alpha;
    }
  }
}

function shareColumn(model: StackModel, frame: PlotFrame, colors: Float64Array, boundaries: Float64Array, changes: Float64Array,
  image: ImageData, column: number, hour: number, end: number, index: number): number {
  changes.fill(0);
  const height = frame.height; const step = end - hour;
  const bottomOfStack = Math.max(0, Math.min(height, frame.toY(0)));
  const yScale = frame.plotHeight / Math.max(1e-12, frame.y[1] - frame.y[0]);
  let coverage = 0;
  while (hour < end) {
    while (index + 1 < model.hours.length && model.hours[index + 1] <= hour) index++;
    const before = !model.hours.length || hour < model.hours[0];
    const until = Math.min(end, before ? (model.hours[0] ?? end) : (model.hours[index + 1] ?? end));
    const edges = before ? null : model.edges[index];
    if (edges) {
      const weight = (until - hour) / step;
      coverage += weight;
      for (let band = 0; band < edges.length; band++) {
        const value = edges[band].high;
        const y = frame.log ? frame.toY(value) : frame.plotTop + frame.plotHeight - (value - frame.y[0]) * yScale;
        const top = Math.max(0, Math.min(height, y));
        if (top < height) addBoundary(changes, boundaries, top, weight, band);
      }
    }
    hour = until;
  }
  if (coverage) {
    if (bottomOfStack < height) addBoundary(changes, colors, bottomOfStack, -coverage, 0);
    writeColumn(changes, image, column);
  }
  return index;
}

function bandColors(bands: readonly ShareBand[], createCanvas: () => HTMLCanvasElement): Uint8ClampedArray {
  if (!bands.length) return new Uint8ClampedArray();
  const canvas = createCanvas(); canvas.width = bands.length; canvas.height = 1;
  const ctx = canvas.getContext('2d', { willReadFrequently: true })!;
  bands.forEach((band, index) => {
    ctx.fillStyle = band.color; ctx.globalAlpha = band.alpha ?? 1;
    ctx.fillRect(index, 0, 1, 1);
  });
  return ctx.getImageData(0, 0, bands.length, 1).data;
}

function changedHour(before: StackModel, after: StackModel): number {
  for (let index = 0; index < Math.max(before.hours.length, after.hours.length); index++) {
    const a = before.edges[index]; const b = after.edges[index];
    if (before.hours[index] !== after.hours[index] || before.breakBefore[index] !== after.breakBefore[index]
      || (a === null) !== (b === null) || a?.length !== b?.length
      || a?.some((edge, band) => edge.low !== b![band].low || edge.high !== b![band].high)) {
      return Math.min(before.hours[index] ?? Infinity, after.hours[index] ?? Infinity);
    }
  }
  return Infinity;
}

export interface ShareLayer extends ChartLayer { update(model: StackModel, extent: readonly [number, number]): void }
type EdgeRaster = { first: number; end: number; key: string; canvas: HTMLCanvasElement };

/** Stable device-resolution tiles. Dense columns integrate exact coverage; sparse
 * polygons have at most two path vertices per device column plus run boundaries.
 * Playback only projects/clips cached pixels; data changes dirty their suffix. */
export function shareLayer(model: StackModel, bands: readonly ShareBand[], extent: readonly [number, number],
  createCanvas = () => document.createElement('canvas')): ShareLayer {
  let key = ''; let tileHours = 0;
  let colors: Uint8ClampedArray | null = null;
  let frontier: EdgeRaster | null = null; let leftEdge: EdgeRaster | null = null;
  const tiles = new Map<number, { canvas: HTMLCanvasElement; dirty: number; dirtyEnd: number }>();
  return { update(next, nextExtent) {
    if (next === model && nextExtent[0] === extent[0] && nextExtent[1] === extent[1]) return;
    const dirty = Math.min(changedHour(model, next), extent[1], nextExtent[1]);
    if (extent[0] !== nextExtent[0]) tiles.clear();
    else for (const [index, tile] of tiles) {
      if (dirty < extent[0] + (index + 1) * tileHours) {
        tile.dirty = Math.min(tile.dirty, dirty);
        tile.dirtyEnd = Math.max(tile.dirtyEnd, Math.max(extent[1], nextExtent[1]) + 1);
      }
    }
    if (frontier && dirty < frontier.end) frontier = null;
    if (leftEdge && dirty < leftEdge.end) leftEdge = null;
    model = next; extent = nextExtent;
  }, draw(ctx, frame, _tokens, state) {
    const from = Math.max(frame.x[0], extent[0]);
    const to = Math.min(frame.x[1], extent[1]);
    const until = Math.min(to, state.playhead ?? to);
    if (until < from) return;
    const transform = ctx.getTransform();
    const scaleX = transform.a; const scaleY = transform.d;
    const width = Math.ceil(frame.plotWidth * scaleX);
    const height = Math.ceil(frame.plotHeight * scaleY);
    const step = 2 ** Math.floor(Math.log2((frame.x[1] - frame.x[0]) / width));
    tileHours = width * step;
    const nextKey = `${width}:${height}:${step}:${frame.y[0]}:${frame.y[1]}:${frame.log}`;
    if (key !== nextKey) {
      key = nextKey; tiles.clear();
    }
    const first = Math.floor((from - extent[0]) / tileHours);
    const last = Math.ceil((to - extent[0]) / tileHours) - 1;
    const wholeFirst = extent[0] + Math.ceil((from - extent[0]) / step) * step;
    const frontierFirst = extent[0] + Math.floor((until - extent[0]) / step) * step;
    const completed = Math.max(wholeFirst, frontierFirst);
    for (const index of tiles.keys()) if (index < first || index > last) tiles.delete(index);
    ctx.save();
    ctx.beginPath();
    ctx.rect(frame.toX(wholeFirst), frame.plotTop, frame.toX(completed) - frame.toX(wholeFirst), frame.plotHeight);
    ctx.clip();
    for (let index = first; index <= last; index++) {
      const start = extent[0] + index * tileHours;
      const end = start + tileHours;
      let tile = tiles.get(index);
      if (!tile) {
        const canvas = createCanvas(); canvas.width = width; canvas.height = height;
        tile = { canvas, dirty: start, dirtyEnd: end }; tiles.set(index, tile);
      }
      if (tile.dirty < end) {
        const target = tile.canvas.getContext('2d', { willReadFrequently: true })!;
        const column = Math.max(0, Math.floor((tile.dirty - start) / step));
        const lastColumn = Math.min(width, Math.ceil((tile.dirtyEnd - start) / step));
        target.clearRect(column, 0, lastColumn - column, height);
        const projection = plotFrame(width, height,
          { left: 0, right: 0, top: 0, bottom: 0 }, [start, end], frame.y, frame.log);
        const from = start + column * step;
        const samples = heldIndex(model.hours, end) - heldIndex(model.hours, from) + 1;
        const deviceColumns = (lastColumn - column) * step / (frame.x[1] - frame.x[0]) * width;
        if (samples * 2 <= deviceColumns) {
          drawShareBands(target, projection, shareSegments(model, from, Math.min(end, extent[1])), bands);
          target.putImageData(target.getImageData(column, 0, lastColumn - column, height), column, 0);
        } else {
          colors ??= bandColors(bands, createCanvas);
          target.putImageData(sharePixels(target, model, projection, colors, column, lastColumn), column, 0);
        }
        tile.dirty = Infinity; tile.dirtyEnd = -Infinity;
      }
      const left = Math.max(0, (wholeFirst - start) / step);
      const right = Math.min(width, (completed - start) / step);
      if (right > left) {
        const x = frame.toX(start + left * step);
        ctx.drawImage(tile.canvas, left, 0, right - left, height, x, frame.plotTop,
          frame.toX(start + right * step) - x, height / scaleY);
      }
    }
    ctx.restore();
    const edge = (cached: EdgeRaster | null, start: number, end: number): EdgeRaster => {
      if (!cached || cached.first !== start || cached.end !== end || cached.key !== key) {
        const canvas = cached?.canvas ?? createCanvas(); canvas.width = 1; canvas.height = height;
        const target = canvas.getContext('2d', { willReadFrequently: true })!;
        colors ??= bandColors(bands, createCanvas);
        const projection = plotFrame(1, height, { left: 0, right: 0, top: 0, bottom: 0 }, [start, end], frame.y, frame.log);
        target.putImageData(sharePixels(target, model, projection, colors), 0, 0);
        cached = { first: start, end, key, canvas };
      }
      ctx.drawImage(cached.canvas, frame.toX(start), frame.plotTop, frame.toX(end) - frame.toX(start), height / scaleY);
      return cached;
    };
    if (wholeFirst > from && until > from) leftEdge = edge(leftEdge, from, Math.min(wholeFirst, until));
    if (frontierFirst >= wholeFirst && until > frontierFirst) {
      frontier = edge(frontier, frontierFirst, until);
    }
  } };
}

/** Shares read as the chart draws them: the runs up to the head (`read(head)`, the head's bin at the followed
 * values), the layer taking each new set in place, so a tile landing or the head moving repaints without a render. */
export interface LiveShares { read(head: number | null): readonly ShareRun[]; revision(): unknown; subscribe(listener: () => void): () => void }

/** `layer` fed from `live` as it draws: a set of runs not seen before is stacked and taken in place (its changed
 * suffix redrawn), the same set draws as held. */
export function liveShareLayer(layer: ShareLayer, live: LiveShares, bands: number, extent: () => readonly [number, number],
  summaries?: () => readonly ChartSummary[] | undefined): ChartLayer {
  let seen: readonly ShareRun[] | null = null;
  return {
    draw(ctx, frame, tokens, state) {
      const runs = live.read(state.playhead === null ? null : Math.min(frame.x[1], state.playhead));
      if (runs !== seen) {
        seen = runs;
        layer.update(shareStack(summaryShareRuns(runs, summaries?.()), bands), extent());
      }
      layer.draw(ctx, frame, tokens, state);
    },
  };
}
