export class VirtualListIndex {
  private tree: Float64Array;
  private heights: Float64Array;
  private gap: number;
  readonly count: number;
  constructor(count: number, estimate: number, gap = 0) {
    this.count = count;
    this.gap = Number.isFinite(gap) && gap > 0 ? gap : 0;
    this.tree = new Float64Array(count + 1); this.heights = new Float64Array(count).fill(estimate);
    for (let i = 1; i <= count; i++) this.tree[i] = (i & -i) * estimate;
  }
  set(index: number, height: number): boolean {
    if (index < 0 || index >= this.count || !(height > 0) || this.heights[index] === height) return false;
    const delta = height - this.heights[index]; this.heights[index] = height;
    for (let i = index + 1; i <= this.count; i += i & -i) this.tree[i] += delta;
    return true;
  }
  setGap(gap: number): boolean {
    if (!Number.isFinite(gap) || gap < 0 || gap === this.gap) return false;
    this.gap = gap; return true;
  }
  offset(end: number): number {
    let sum = 0; for (let i = Math.min(this.count, end); i > 0; i -= i & -i) sum += this.tree[i];
    return sum + Math.max(0, Math.min(this.count - 1, end)) * this.gap;
  }
  rowAt(pixel: number): number {
    let index = 0, sum = 0, bit = 1; while (bit * 2 <= this.count) bit *= 2;
    for (; bit; bit >>>= 1) {
      const next = index + bit;
      if (next <= this.count && sum + this.tree[next] + Math.min(next, this.count - 1) * this.gap <= pixel) { index = next; sum += this.tree[next]; }
    }
    return Math.min(this.count, index);
  }
  range(top: number, height: number, overscan = 3): [number, number] {
    return [Math.max(0, this.rowAt(Math.max(0, top)) - overscan), Math.min(this.count, this.rowAt(Math.max(0, top + height)) + overscan + 1)];
  }
}
