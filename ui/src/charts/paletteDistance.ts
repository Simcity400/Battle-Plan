import { rgbToOklch } from '../shell/appearance/color.ts';
import type { Rgb } from '../shell/appearance/color';

type Point = readonly [number, number, number];
interface Node { point: Point; left: Node | null; right: Node | null }
interface Bucket { points: Point[]; tree: Node }

function point(color: Rgb): Point {
  const { l, c, h } = rgbToOklch(color);
  return [l, c * Math.cos(h * Math.PI / 180), c * Math.sin(h * Math.PI / 180)];
}

function apart(a: Point, b: Point): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]) * 100;
}

export function colourDistance(first: Rgb, second: Rgb): number {
  return apart(point(first), point(second));
}

function build(points: readonly Point[], axis: number): Node {
  const sorted = [...points].sort((a, b) => a[axis] - b[axis]);
  const middle = Math.floor(sorted.length / 2), next = (axis + 1) % 3;
  return {
    point: sorted[middle],
    left: middle ? build(sorted.slice(0, middle), next) : null,
    right: middle + 1 < sorted.length ? build(sorted.slice(middle + 1), next) : null,
  };
}

function nearest(node: Node | null, query: Point, axis: number, best: number): number {
  if (!node || Number.isNaN(best)) return best;
  best = Math.min(best, apart(query, node.point));
  const delta = query[axis] - node.point[axis], next = (axis + 1) % 3;
  const before = delta <= 0;
  best = nearest(before ? node.left : node.right, query, next, best);
  const bound = Math.abs(delta) * 100;
  if (bound <= best + Math.abs(best) * Number.EPSILON * 4 + Number.MIN_VALUE) {
    best = nearest(before ? node.right : node.left, query, next, best);
  }
  return best;
}

export class NearestColours {
  private readonly buckets: (Bucket | null)[] = [];
  private readonly seen = new Set<string>();
  private readonly special: Point[] = [];

  constructor(colors: readonly Rgb[] = []) {
    for (const color of colors) this.add(color);
  }

  add(color: Rgb): void {
    const value = point(color);
    if (!value.every(Number.isFinite)) { this.special.push(value); return; }
    const key = value.join(',');
    if (this.seen.has(key)) return;
    this.seen.add(key);
    let points = [value], level = 0;
    while (this.buckets[level]) {
      points = [...this.buckets[level]!.points, ...points];
      this.buckets[level++] = null;
    }
    this.buckets[level] = { points, tree: build(points, 0) };
  }

  gap(color: Rgb): number {
    const query = point(color);
    let best = Infinity;
    for (const other of this.special) best = Math.min(best, apart(query, other));
    const finite = query.every(Number.isFinite);
    for (const bucket of this.buckets) {
      if (!bucket) continue;
      if (finite) best = nearest(bucket.tree, query, 0, best);
      else for (const other of bucket.points) best = Math.min(best, apart(query, other));
    }
    return best;
  }
}
