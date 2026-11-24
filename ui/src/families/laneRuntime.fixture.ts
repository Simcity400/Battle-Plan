import assert from 'node:assert/strict';
import type { Picture } from '../dashboard/follow/facts.ts';
import type { LaneView, Painted, Sub, Value } from '../draw/lane.ts';

export interface RecordedCanvas {
  readonly width: number;
  readonly height: number;
  readonly frames: string[][];
}

const METRICS = (text: unknown) => {
  const width = String(text).length * 7;
  return { width, actualBoundingBoxAscent: 8, actualBoundingBoxDescent: 2, actualBoundingBoxLeft: 0, actualBoundingBoxRight: width, fontBoundingBoxAscent: 9, fontBoundingBoxDescent: 3 };
};

function shown(value: unknown): string {
  if (typeof value === 'number') return Number.isInteger(value) ? String(value) : value.toFixed(2);
  if (typeof value === 'string') return JSON.stringify(value);
  if (value === null || value === undefined || typeof value === 'boolean') return String(value);
  return '[object]';
}

function recordingContext(canvas: RecordedCanvas): unknown {
  const props = new Map<PropertyKey, unknown>([['canvas', canvas], ['font', '10px sans-serif'], ['globalAlpha', 1], ['lineWidth', 1]]);
  let depth = 0;
  let frame: string[] | null = null;
  const log = (line: string) => { (frame ?? (frame = [], canvas.frames.push(frame), frame)).push(line); };
  const special: Record<string, (...args: unknown[]) => unknown> = {
    measureText: text => METRICS(text),
    save: () => {
      if (depth === 0) { frame = []; canvas.frames.push(frame); }
      depth++;
      log('save');
    },
    restore: () => {
      log('restore');
      depth = Math.max(0, depth - 1);
      if (depth === 0) frame = null;
    },
    createLinearGradient: (...args) => { log(`gradient(${args.map(shown).join(',')})`); return { addColorStop: (at: number, colour: string) => log(`stop(${shown(at)},${colour})`) }; },
    createRadialGradient: (...args) => { log(`radial(${args.map(shown).join(',')})`); return { addColorStop: (at: number, colour: string) => log(`stop(${shown(at)},${colour})`) }; },
    createPattern: () => { log('pattern'); return {}; },
    getTransform: () => ({ a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }),
    isPointInPath: () => false,
    getLineDash: () => [],
  };
  return new Proxy({}, {
    get: (_target, prop) => {
      if (typeof prop === 'string' && special[prop]) return special[prop];
      if (props.has(prop)) return props.get(prop);
      if (typeof prop === 'symbol') return undefined;
      return (...args: unknown[]) => log(`${prop}(${args.map(shown).join(',')})`);
    },
    set: (_target, prop, value) => {
      props.set(prop, value);
      log(`${String(prop)}=${shown(value)}`);
      return true;
    },
  });
}

export function installRecordingCanvas(): { readonly canvases: RecordedCanvas[]; restore(): void } {
  const canvases: RecordedCanvas[] = [];
  const previous = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas = class {
    width: number;
    height: number;
    frames: string[][] = [];
    private ctx: unknown = null;
    constructor(width: number, height: number) {
      this.width = width;
      this.height = height;
      canvases.push(this);
    }
    getContext() { return this.ctx ??= recordingContext(this); }
    transferToImageBitmap() { return { close() {} }; }
  };
  return { canvases, restore: () => { (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas = previous; } };
}

export interface LaneStep {
  readonly label: string;
  readonly binding: unknown;
  readonly hour: number;
  readonly subs?: readonly Sub[];
  readonly pictures?: readonly Picture<Value>[];
  readonly inspect?: string | null;
  readonly speed?: number;
  readonly check?: boolean;
}

export interface LaneDriver {
  paint(step: LaneStep): Painted;
  picture(): readonly string[];
  readonly state: Map<string, unknown>;
}

export function driveLane(view: LaneView, canvases: RecordedCanvas[], surface = { width: 320, height: 200, dpr: 1 }): LaneDriver {
  const state = new Map<string, unknown>();
  let seq = 0;
  let canvas: RecordedCanvas | null = null;
  return {
    state,
    paint(step) {
      const before = canvases.length;
      const painted = view.paint({
        widget: 1,
        subs: step.subs ?? [],
        pictures: step.pictures ?? [],
        model: step.binding,
        hour: step.hour,
        speed: step.speed ?? 0,
        binding: { generation: 1, surfaces: [surface], offset: 0, pin: null },
        inspect: step.inspect ?? null,
        now: 1000,
      }, state, () => `c${++seq}`);
      if (canvases.length > before) canvas = canvases[before]!;
      return painted;
    },
    picture: () => (canvas?.frames.at(-1) ?? []),
  };
}

const GATED = new Set(['mirror', 'tooltip', 'focusTooltip']);

function fieldValues(painted: Painted, speed: number): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [name, entry] of Object.entries(painted.fields)) {
    if (speed > 0 && GATED.has(name)) continue;
    out[name] = JSON.parse(JSON.stringify(entry.value, (_k, v) => (typeof v === 'bigint' ? `${v}n` : v)));
  }
  return out;
}

export function assertFreshPaint(makeView: () => LaneView, steps: readonly LaneStep[], surface?: { width: number; height: number; dpr: number }): void {
  const recording = installRecordingCanvas();
  try {
    const played = driveLane(makeView(), recording.canvases, surface);
    for (const step of steps) {
      const painted = played.paint(step);
      if (!step.check) continue;
      const fresh = driveLane(makeView(), recording.canvases, surface);
      const freshPainted = fresh.paint(step);
      assert.deepEqual(played.picture(), fresh.picture(), `${step.label}: the picture equals a fresh paint`);
      assert.deepEqual(fieldValues(painted, step.speed ?? 0), fieldValues(freshPainted, step.speed ?? 0), `${step.label}: the fields equal a fresh paint`);
      assert.equal(painted.mode, freshPainted.mode, `${step.label}: the mode equals a fresh paint`);
      assert.equal(painted.nextChangeAt, freshPainted.nextChangeAt, `${step.label}: the proven interval equals a fresh paint`);
    }
  } finally {
    recording.restore();
  }
}
