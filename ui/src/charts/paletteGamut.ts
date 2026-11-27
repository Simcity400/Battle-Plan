import { luminance, parseColor, toHex } from '../shell/appearance/color.ts';
import type { Rgb } from '../shell/appearance/color';

const RED = Float64Array.from({ length: 256 }, (_, r) => luminance({ r, g: 0, b: 0 }));
const GREEN = Float64Array.from({ length: 256 }, (_, g) => luminance({ r: 0, g, b: 0 }));
const BLUE = Float64Array.from({ length: 256 }, (_, b) => luminance({ r: 0, g: 0, b }));

function blueBound(value: number, after: boolean): number {
  let lo = 0, hi = 256;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (BLUE[mid]! < value || after && BLUE[mid] === value) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

function gcd(a: number, b: number): number {
  while (b) [a, b] = [b, a % b];
  return a;
}

export class ContrastGamut {
  readonly size: number;
  private readonly counts = new Uint32Array(256 * 256 + 1);
  private readonly excluded = new Set<number>();
  private readonly contrasts: readonly { readonly light: number; readonly ratio: number }[];
  private readonly bands: readonly (readonly [number, number])[];
  private readonly stride: number;

  constructor(constraints: readonly { readonly background: Rgb; readonly ratio: number }[], excluded: Iterable<string>) {
    this.contrasts = constraints.map(({ background, ratio }) => ({ light: luminance(background), ratio }));
    let bands: [number, number][] = [[0, 1]];
    for (const { light, ratio } of this.contrasts) {
      const low = (light + 0.05) / ratio - 0.05;
      const high = (light + 0.05) * ratio - 0.05;
      bands = bands.flatMap(([from, to]) => {
        const kept: [number, number][] = [];
        if (from <= Math.min(to, low)) kept.push([from, Math.min(to, low)]);
        if (Math.max(from, high) <= to) kept.push([Math.max(from, high), to]);
        return kept;
      });
    }
    this.bands = bands;
    for (const hex of excluded) {
      const rgb = parseColor(hex);
      if (rgb && [rgb.r, rgb.g, rgb.b].every(Number.isInteger)) this.excluded.add((rgb.r << 16) | (rgb.g << 8) | rgb.b);
    }
    const omitted = [...this.excluded].sort((a, b) => a - b);
    let nextOmitted = 0;
    for (let row = 0; row < 256 * 256; row++) {
      const base = RED[row >>> 8]! + GREEN[row & 255]!;
      const ranges = this.blueRanges(base);
      let count = 0;
      for (let i = 0; i < ranges.length; i += 2) count += ranges[i + 1]! - ranges[i]! + 1;
      while (nextOmitted < omitted.length && omitted[nextOmitted]! >>> 8 === row) {
        const blue = omitted[nextOmitted++]! & 255;
        if (this.accepts(base + BLUE[blue]!)) count--;
      }
      this.counts[row + 1] = this.counts[row]! + count;
    }
    this.size = this.counts[this.counts.length - 1]!;
    let stride = Math.max(1, Math.floor(this.size * 0.6180339887498949));
    while (this.size && gcd(stride, this.size) !== 1) stride++;
    this.stride = stride;
  }

  private accepts(light: number): boolean {
    return this.contrasts.every(other => (Math.max(light, other.light) + 0.05) / (Math.min(light, other.light) + 0.05) >= other.ratio);
  }

  private blueRanges(base: number): number[] {
    const result: number[] = [];
    for (const [from, to] of this.bands) {
      let first = blueBound(from - base, false);
      let last = blueBound(to - base, true) - 1;
      if (first > 0 && this.accepts(base + BLUE[first - 1]!)) first--;
      if (last < 255 && this.accepts(base + BLUE[last + 1]!)) last++;
      if (first < 256 && !this.accepts(base + BLUE[first]!)) first++;
      if (last >= 0 && !this.accepts(base + BLUE[last]!)) last--;
      if (first > last) continue;
      if (result.length && first <= result[result.length - 1]! + 1) result[result.length - 1] = Math.max(last, result[result.length - 1]!);
      else result.push(first, last);
    }
    return result;
  }

  at(index: number): string {
    let rank = index * this.stride % this.size;
    let lo = 0, hi = this.counts.length - 1;
    while (lo + 1 < hi) {
      const mid = (lo + hi) >>> 1;
      if (this.counts[mid]! <= rank) lo = mid;
      else hi = mid;
    }
    rank -= this.counts[lo]!;
    const r = lo >>> 8, g = lo & 255;
    const ranges = this.blueRanges(RED[r]! + GREEN[g]!);
    for (let i = 0; i < ranges.length; i += 2) {
      for (let b = ranges[i]!; b <= ranges[i + 1]!; b++) {
        if (this.excluded.has((r << 16) | (g << 8) | b)) continue;
        if (rank-- === 0) return toHex({ r, g, b });
      }
    }
    throw new RangeError('Colour index is outside the contrast-valid RGB gamut');
  }
}
