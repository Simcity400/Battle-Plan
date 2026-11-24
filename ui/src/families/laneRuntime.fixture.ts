import assert from 'node:assert/strict';
import type { Picture } from '../dashboard/follow/facts.ts';
import { PayloadSender, PayloadStore } from '../dashboard/follow/payloads.ts';
import type { LaneView, Painted, Sub, Value } from '../draw/lane.ts';
import type { FamilyTheme } from './contract.ts';

export const LANE_THEME: FamilyTheme = {
  surface: '#16181d', raised: '#1e2128', dark: true, text: '#eef1f6', muted: '#8c95a6', line: '#2e3440', lineSoft: '#222630',
  accent: '#4f86f7', blue: '#4f86f7', red: '#e5534b', shortage: '#ff6b6f', amber: '#d29922', green: '#3fb950', grey: '#8c95a6',
  hatch: '#6e7681', hatchEdge: '#484f58', font: 'Inter', fontSize: { small: 11, body: 12 }, lineWidth: 1, hatchSpacing: 6,
};

export interface RecordedCanvas {
  readonly width: number;
  readonly height: number;
  readonly frames: string[][];
  readonly marks: Map<string, string>;
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
  let t = { a: 1, d: 1, e: 0, f: 0 };
  const stack: (typeof t)[] = [];
  const px = (x: unknown) => Math.round(t.e + Number(x) * t.a);
  const py = (y: unknown) => Math.round(t.f + Number(y) * t.d);
  const cover = (x: unknown, y: unknown, w: unknown, h: unknown) => {
    const x0 = px(x), y0 = py(y), x1 = px(Number(x) + Number(w)), y1 = py(Number(y) + Number(h));
    for (const key of [...canvas.marks.keys()]) {
      const [, mx, my] = key.split(':');
      const kx = Number(mx), ky = Number(my);
      if (kx >= Math.min(x0, x1) && kx < Math.max(x0, x1) && ky >= Math.min(y0, y1) && ky < Math.max(y0, y1)) canvas.marks.delete(key);
    }
    return [x0, y0, x1 - x0, y1 - y0] as const;
  };
  const log = (line: string) => { (frame ?? (frame = [], canvas.frames.push(frame), frame)).push(line); };
  const special: Record<string, (...args: unknown[]) => unknown> = {
    measureText: text => METRICS(text),
    save: () => {
      if (depth === 0) { frame = []; canvas.frames.push(frame); }
      depth++;
      stack.push({ ...t });
      log('save');
    },
    restore: () => {
      log('restore');
      t = stack.pop() ?? t;
      depth = Math.max(0, depth - 1);
      if (depth === 0) frame = null;
    },
    setTransform: (a, _b, _c, d, e, f) => { t = { a: Number(a), d: Number(d), e: Number(e), f: Number(f) }; log('setTransform'); },
    translate: (x, y) => { t = { ...t, e: t.e + Number(x) * t.a, f: t.f + Number(y) * t.d }; log(`translate(${shown(x)},${shown(y)})`); },
    scale: (x, y) => { t = { ...t, a: t.a * Number(x), d: t.d * Number(y) }; log('scale'); },
    clearRect: (x, y, w, h) => { cover(x, y, w, h); log('clearRect'); },
    fillRect: (x, y, w, h) => {
      const [x0, y0, ww, hh] = cover(x, y, w, h);
      canvas.marks.set(`r:${x0}:${y0}`, `${String(props.get('fillStyle'))}:${ww}x${hh}`);
      log('fillRect');
    },
    fillText: (text, x, y) => { canvas.marks.set(`t:${px(x)}:${py(y)}`, `${String(props.get('fillStyle'))}|${String(text)}`); log(`fillText(${shown(text)})`); },
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
    marks = new Map<string, string>();
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

export class LaneTransport {
  readonly sent = { payloads: 0 };
  private readonly store = new PayloadStore();
  private readonly sender = new PayloadSender(message => {
    this.sent.payloads += message.put?.length ?? 0;
    this.store.apply(structuredClone(message));
  });
  private readonly held = new Map<string, ReadonlySet<number>>();

  carry<T>(binding: T, placement = 'placement'): T {
    const encoded = this.sender.acquire(binding);
    const decoded = this.store.resolve(structuredClone(encoded.wire)) as T;
    const was = this.held.get(placement);
    this.held.set(placement, encoded.ids);
    if (was) this.sender.release(was);
    return decoded;
  }
}

export interface LaneStep {
  readonly label: string;
  readonly binding: unknown;
  readonly hour: number;
  readonly subs?: readonly Sub[];
  readonly pictures?: readonly Picture<Value>[];
  readonly inspect?: string | null;
  readonly speed?: number;
  readonly seek?: number;
  readonly owner?: string;
  readonly generation?: number;
  readonly now?: number;
  readonly check?: boolean;
}

export interface LaneDriver {
  paint(step: LaneStep): Painted;
  picture(): readonly string[];
  model(): unknown;
  readonly state: Map<string, unknown>;
  readonly transport: LaneTransport;
}

export function driveLane(view: LaneView, canvases: RecordedCanvas[], surface = { width: 320, height: 200, dpr: 1 }, transport = new LaneTransport(), placement = 'placement'): LaneDriver {
  const state = new Map<string, unknown>();
  let seq = 0;
  let canvas: RecordedCanvas | null = null;
  let from: unknown;
  let to: unknown;
  const model = (binding: unknown) => {
    if (to === undefined || from !== binding) {
      from = binding;
      to = transport.carry(binding, placement);
    }
    return to;
  };
  return {
    state,
    transport,
    model: () => to,
    paint(step) {
      const before = canvases.length;
      const painted = view.paint({
        widget: 1,
        subs: step.subs ?? [],
        pictures: step.pictures ?? [],
        model: model(step.binding),
        ...(step.owner !== undefined ? { owner: step.owner } : {}),
        hour: step.hour,
        seek: step.seek ?? 0,
        speed: step.speed ?? 0,
        binding: { generation: step.generation ?? 1, surfaces: [surface], offset: 0, pin: null, ...(step.owner !== undefined ? { owner: step.owner } : {}) },
        inspect: step.inspect ?? null,
        now: step.now ?? 1000,
      }, state, () => `c${++seq}`);
      if (canvases.length > before) canvas = canvases[before]!;
      return painted;
    },
    picture: () => {
      if (!canvas) return [];
      const shown: string[] = [];
      for (const [key, value] of canvas.marks) {
        const [, x, y] = key.split(':');
        if (Number(x) >= 0 && Number(x) < canvas.width && Number(y) >= 0 && Number(y) < canvas.height) shown.push(`${key}=${value}`);
      }
      return shown.sort();
    },
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
    let seek = 0;
    for (const step of steps) {
      if (step.check) seek++;
      const painted = played.paint({ ...step, seek: step.seek ?? seek });
      if (!step.check) continue;
      const fresh = driveLane(makeView(), recording.canvases, surface);
      const freshPainted = fresh.paint(step);
      assert.ok(fresh.picture().length > 0, `${step.label}: a fresh paint draws something`);
      assert.deepEqual(played.picture(), fresh.picture(), `${step.label}: the picture equals a fresh paint`);
      assert.deepEqual(fieldValues(painted, step.speed ?? 0), fieldValues(freshPainted, step.speed ?? 0), `${step.label}: the fields equal a fresh paint`);
      assert.equal(painted.mode, freshPainted.mode, `${step.label}: the mode equals a fresh paint`);
      assert.equal(painted.nextChangeAt, freshPainted.nextChangeAt, `${step.label}: the proven interval equals a fresh paint`);
    }
  } finally {
    recording.restore();
  }
}

export function assertQuietFrames(makeView: () => LaneView, step: LaneStep, hours = 24, surface?: { width: number; height: number; dpr: number }): number {
  const recording = installRecordingCanvas();
  try {
    const view = makeView();
    const lane = driveLane(view, recording.canvases, surface);
    const first = lane.paint({ ...step, speed: step.speed ?? 24 });
    const boundary = first.nextChangeAt ?? Infinity;
    const drawn = recording.canvases.reduce((sum, canvas) => sum + canvas.frames.length, 0);
    const contents = (painted: Painted) => [painted.surfaces.map(surface => surface?.content ?? null), Object.fromEntries(Object.entries(painted.fields).map(([name, entry]) => [name, entry.content]))];
    const scene = { widget: 1, subs: step.subs ?? [], pictures: step.pictures ?? [], model: lane.model() };
    const last = Math.min(boundary - 1, step.hour + hours);
    for (let hour = step.hour; hour <= last; hour++) {
      if (view.next) assert.equal(view.next(scene, hour, lane.state), boundary, `${step.label}: next() at hour ${hour} answers the proven boundary`);
      for (const speed of [step.speed ?? 24, 0]) {
        const painted = lane.paint({ ...step, hour, speed });
        assert.deepEqual(contents(painted), contents(first), `${step.label}: a quiet frame at hour ${hour} (speed ${speed}) keeps every content tag`);
      }
    }
    assert.equal(recording.canvases.reduce((sum, canvas) => sum + canvas.frames.length, 0), drawn, `${step.label}: quiet, steady and paused frames draw nothing`);
    return boundary;
  } finally {
    recording.restore();
  }
}
