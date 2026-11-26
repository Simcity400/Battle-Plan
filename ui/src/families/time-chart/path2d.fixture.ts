export type PathOp = readonly ['M' | 'L', number, number] | readonly ['Z'];

export const pathCalls = { count: 0 };
export const pathAllocations = { count: 0 };
export const addPathCalls = { count: 0 };

export class RecordedPath2D {
  readonly ops: PathOp[] = [];

  constructor(from?: RecordedPath2D) {
    pathAllocations.count++;
    if (from) this.ops.push(...from.ops);
  }

  moveTo(x: number, y: number): void {
    pathCalls.count++;
    this.ops.push(['M', x, y]);
  }

  lineTo(x: number, y: number): void {
    pathCalls.count++;
    this.ops.push([this.ops.length ? 'L' : 'M', x, y]);
  }

  closePath(): void {
    this.ops.push(['Z']);
  }

  rect(x: number, y: number, w: number, h: number): void {
    this.ops.push(['M', x, y], ['L', x + w, y], ['L', x + w, y + h], ['L', x, y + h], ['Z']);
  }

  addPath(path: RecordedPath2D, m: { a?: number; b?: number; c?: number; d?: number; e?: number; f?: number } = {}): void {
    addPathCalls.count++;
    const { a = 1, b = 0, c = 0, d = 1, e = 0, f = 0 } = m;
    for (const op of path.ops) this.ops.push(op[0] === 'Z' ? op : [op[0], a * op[1] + c * op[2] + e, b * op[1] + d * op[2] + f]);
  }

  subpaths(): [number, number][][] {
    const out: [number, number][][] = [];
    for (const op of this.ops) {
      if (op[0] === 'M') out.push([[op[1], op[2]]]);
      else if (op[0] === 'L') out[out.length - 1]!.push([op[1], op[2]]);
    }
    return out;
  }
}

if (!('Path2D' in globalThis)) (globalThis as { Path2D?: unknown }).Path2D = RecordedPath2D;
