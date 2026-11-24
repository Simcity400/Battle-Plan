import assert from 'node:assert/strict';
import test from 'node:test';
import { Page, type Bitmap, type PageMessage } from '../dashboard/follow/page.ts';
import { Lane } from '../draw/lane.ts';
import type { BoundedReadView, FamilyProjection } from './contract.ts';
import { createLaneView, type LaneBinding, type LaneFamily } from './laneRuntime.ts';
import { assertFreshPaint, assertQuietFrames, driveLane, installRecordingCanvas, LANE_THEME as THEME } from './laneRuntime.fixture.ts';
import { bindFamilyManifest, registerFamilyProjection } from './registry.ts';
import { IdentityIndex } from './identity/index.ts';
import type { IdentitySnapshot } from './identity/types.ts';

type Row = { readonly key: string; readonly v: number };
type Options = { readonly order: 'up' | 'down'; readonly name?: string };
type Model = { readonly hour: number; readonly bucket: number; readonly rows: readonly Row[]; readonly name: string };
type Layout = { readonly rows: readonly { readonly key: string; readonly y: number; readonly v: number }[] };
type Binding = LaneBinding<Options> & { readonly family: 'test-lane' };

const counts = { project: 0, nextChange: 0, layout: 0, paint: 0, tooltip: 0, mirror: 0 };
const reset = () => { for (const key of Object.keys(counts) as (keyof typeof counts)[]) counts[key] = 0; };

registerFamilyProjection<FamilyProjection<Model, Options>>({
  kind: 'fixture',
  family: 'test-lane',
  project(view: BoundedReadView, options: Options): Model {
    counts.project++;
    const rows = [...(view.panel<{ rows: Row[] }>('main')?.rows ?? [])];
    const name = options.name ? view.who('country', options.name, view.hour).name : null;
    return { hour: view.hour, bucket: Math.floor(view.hour / 10), rows: options.order === 'up' ? rows.reverse() : rows, name: name?.status === 'ready' ? name.value : options.name ?? '' };
  },
  nextChange(view: BoundedReadView): number {
    counts.nextChange++;
    return Math.floor(view.hour / 10) * 10 + 10;
  },
});

const family: LaneFamily<Binding, Model, Layout> = {
  family: 'test-lane',
  boundary: () => Infinity,
  redraw: () => Infinity,
  status: () => 'picture',
  view: () => [],
  layout(model) {
    counts.layout++;
    return { rows: model.rows.map((row, at) => ({ key: row.key, y: at * 20, v: row.v })) };
  },
  paint(ctx, frame) {
    counts.paint++;
    ctx.clearRect(0, 0, frame.surface.width, frame.surface.height);
    ctx.fillStyle = frame.theme.text;
    for (const row of frame.layout.rows) ctx.fillText(`${row.key} ${row.v} ${frame.model.name} ${frame.model.bucket}`, 0, row.y - frame.binding.scrollTop);
    return false;
  },
  hit: (frame, point) => frame.layout.rows.find(row => point[1] >= row.y && point[1] < row.y + 20)?.key ?? null,
  tooltip(frame, key) {
    counts.tooltip++;
    const row = frame.model.rows.find(item => item.key === key);
    return row ? { hour: frame.hour, rowKey: key, content: { title: key, rows: [] } as never } : null;
  },
  structure: frame => ({ meta: { count: frame.model.rows.length }, nav: frame.layout.rows.map(row => row.key) }),
  mirror: frame => {
    counts.mirror++;
    return frame.model.rows.map(row => `${row.key} ${row.v}`);
  },
  audit: frame => frame.layout.rows.map(row => [row.key, row.v]),
  visibleArt: () => [],
};

const rowsAt = (rows: readonly Row[]) => ({ main: { rows } });
const DOWN: Options = { order: 'down' };
const NO_SOURCES = {};
const binding = (staticPanels: Readonly<Record<string, unknown>>, options: Options = DOWN, extra: Partial<Binding> = {}): Binding => ({
  family: 'test-lane', kind: 'fixture', options, sources: NO_SOURCES, staticPanels, scrollTop: 0, theme: THEME, ...extra,
});

test('a projection’s identity demands are published by the paint that ran it', () => {
  const recording = installRecordingCanvas();
  try {
    const lane = driveLane(createLaneView(family, () => {}), recording.canvases);
    const painted = lane.paint({ label: 'first', binding: binding(rowsAt([{ key: 'A', v: 1 }]), { order: 'down', name: 'GER' }), hour: 12 });
    assert.deepEqual(painted.fields.identityDemands?.value, [['country', 'GER', 12]]);
    const plain = lane.paint({ label: 'no names', binding: binding(rowsAt([{ key: 'A', v: 1 }])), hour: 12 });
    assert.equal(plain.fields.identityDemands, undefined, 'a projection that asked for nothing publishes no demands');
  } finally {
    recording.restore();
  }
});

test('a quiet, steady or paused frame does no projection, layout or drawing, and next() answers from the held model', () => {
  reset();
  const recording = installRecordingCanvas();
  try {
    const view = createLaneView(family, () => {});
    const lane = driveLane(view, recording.canvases);
    const bound = binding(rowsAt([{ key: 'A', v: 1 }, { key: 'B', v: 2 }]));
    const first = lane.paint({ label: 'first', binding: bound, hour: 12, speed: 24 });
    assert.equal(first.nextChangeAt, 20, 'the proven interval reaches the projection’s next change');
    assert.deepEqual({ ...counts }, { project: 1, nextChange: 1, layout: 1, paint: 1, tooltip: 0, mirror: 1 });
    const scene = { widget: 1, subs: [], pictures: [], model: bound };
    for (let hour = 13; hour < 20; hour++) assert.equal(view.next!(scene, hour, lane.state), 20);
    const steady = lane.paint({ label: 'steady', binding: bound, hour: 15, speed: 24 });
    const paused = lane.paint({ label: 'paused', binding: bound, hour: 15, speed: 0 });
    assert.deepEqual({ ...counts }, { project: 1, nextChange: 1, layout: 1, paint: 1, tooltip: 0, mirror: 1 }, 'quiet hours cost nothing');
    assert.equal(steady.surfaces[0]!.content, first.surfaces[0]!.content, 'a steady frame keeps its content tag');
    assert.equal(paused.surfaces[0]!.content, first.surfaces[0]!.content);
    lane.paint({ label: 'boundary', binding: bound, hour: 20, speed: 24 });
    assert.equal(counts.project, 2, 'the boundary rebuilds');
  } finally {
    recording.restore();
  }
});

test('a dirty mirror or tooltip arms one wake per boundary, never one per hour', () => {
  reset();
  const set: number[] = [];
  const pending: (() => void)[] = [];
  const timers = {
    set: (fn: () => void, ms: number) => { set.push(ms); pending.push(fn); return pending.length as unknown as ReturnType<typeof setTimeout>; },
    clear: () => {},
  };
  let wakes = 0;
  const recording = installRecordingCanvas();
  try {
    const lane = driveLane(createLaneView(family, () => { wakes++; }, { timers }), recording.canvases);
    lane.paint({ label: 'first', binding: binding(rowsAt([{ key: 'A', v: 1 }])), hour: 12, speed: 24, inspect: 'A' });
    assert.equal(set.length, 0, 'the first paint emits at once');
    for (let hour = 20; hour < 60; hour += 10) lane.paint({ label: `hour ${hour}`, binding: binding(rowsAt([{ key: 'A', v: hour }])), hour, speed: 24, inspect: 'A' });
    assert.equal(set.length, 1, 'several changed hours inside one cadence arm one timer');
    pending[0]!();
    assert.equal(wakes, 1);
  } finally {
    recording.restore();
  }
});

test('after a seek back, a same-hour correction and a reorder the picture equals a fresh paint', () => {
  const atTen = rowsAt([{ key: 'A', v: 1 }, { key: 'B', v: 2 }]);
  const atTwenty = rowsAt([{ key: 'A', v: 5 }, { key: 'B', v: 3 }]);
  const corrected = rowsAt([{ key: 'A', v: 9 }, { key: 'B', v: 2 }]);
  const held = binding(atTwenty);
  assertFreshPaint(() => createLaneView(family, () => {}), [
    { label: 'later bucket', binding: held, hour: 25, speed: 24 },
    { label: 'seek back inside the same facts', binding: held, hour: 15, check: true },
    { label: 'at ten', binding: binding(atTen), hour: 10, check: true },
    { label: 'at twenty', binding: binding(atTwenty), hour: 20, speed: 24 },
    { label: 'seek back', binding: binding(atTen), hour: 10, check: true },
    { label: 'correction', binding: binding(corrected), hour: 10, check: true },
    { label: 'reorder', binding: binding(corrected, { order: 'up' }), hour: 10, check: true },
    { label: 'scroll', binding: binding(corrected, { order: 'up' }, { scrollTop: 20 }), hour: 10, check: true },
  ]);
});

test('a previous frame is supplied only for continuous forward playback of the same placement', () => {
  const seen: (number | null)[] = [];
  const watching: LaneFamily<Binding, Model, Layout> = { ...family, paint: (ctx, frame) => { seen.push(frame.before?.hour ?? null); return family.paint(ctx, frame); } };
  const recording = installRecordingCanvas();
  try {
    const lane = driveLane(createLaneView(watching, () => {}), recording.canvases);
    const rows = binding(rowsAt([{ key: 'A', v: 1 }, { key: 'B', v: 2 }]));
    const swapped = binding(rowsAt([{ key: 'B', v: 3 }, { key: 'A', v: 1 }]));
    const paint = (step: Partial<Parameters<typeof lane.paint>[0]> & { readonly hour: number }) => {
      seen.length = 0;
      lane.paint({ label: 'step', binding: rows, speed: 24, seek: 1, owner: 'a', ...step });
      return seen.at(-1);
    };
    paint({ hour: 12 });
    assert.equal(paint({ hour: 20 }), 12, 'playing on into the next proven interval glides from the frame drawn before');
    assert.equal(paint({ hour: 10, seek: 2 }), null, 'a seek back gives none');
    assert.equal(paint({ hour: 25, seek: 3 }), null, 'a seek forward gives none');
    assert.equal(paint({ hour: 30, seek: 3, speed: 0 }), null, 'a paused frame gives none');
    assert.equal(paint({ hour: 40, seek: 3 }), null, 'nor does the first playing frame after it');
    assert.equal(paint({ hour: 50, seek: 3, owner: 'b' }), null, 'another owner gives none');
    assert.equal(paint({ hour: 60, seek: 3, owner: 'b', binding: binding(rowsAt([{ key: 'A', v: 1 }, { key: 'B', v: 2 }]), { order: 'up' }) }), null, 'another selection gives none');
    assert.equal(paint({ hour: 55, seek: 3, owner: 'b', binding: swapped }), null, 'an earlier hour on the same seek gives none');
    assert.equal(paint({ hour: 70, seek: 3, owner: 'b', binding: swapped }), 55, 'playing on again glides');
  } finally {
    recording.restore();
  }
});

test('the shared quiet-frame check: no drawing or new field between boundaries, and a playing chart’s next hour is a real boundary', () => {
  assert.equal(assertQuietFrames(() => createLaneView(family, () => {}), { label: 'fixture', binding: binding(rowsAt([{ key: 'A', v: 1 }])), hour: 12 }), 20);
  const hourly: LaneFamily<Binding, Model, Layout> = { ...family, redraw: (_model, hour) => hour + 1 };
  assert.equal(assertQuietFrames(() => createLaneView(hourly, () => {}), { label: 'hourly', binding: binding(rowsAt([{ key: 'A', v: 1 }])), hour: 12 }), 13);
});

const settle = async () => { for (let i = 0; i < 8; i++) await null; };

test('a cold projection load walks no history, keeps one waiter per placement, and a failed import fails by name until the host’s retry re-binds it', async () => {
  let release: (value: unknown) => void = () => {};
  let loads = 0;
  bindFamilyManifest([
    { family: 'test-lane', kind: 'cold', loadSpec: () => { loads++; return new Promise(resolve => { release = resolve; }) as never; } },
    { family: 'test-lane', kind: 'broken', loadSpec: () => { loads++; return Promise.reject(new Error('chunk missing')); } },
  ]);
  const recording = installRecordingCanvas();
  try {
    let wakes = 0;
    const view = createLaneView(family, () => { wakes++; });
    const lane = driveLane(view, recording.canvases);
    const cold = { ...binding(rowsAt([])), kind: 'cold' };
    const scene = { widget: 1, subs: [], pictures: [], model: cold };
    for (let hour = 0; hour < 1000; hour++) assert.equal(view.next!(scene, hour, lane.state), null, 'loading locates no change, so the lane walks no history and paints the target');
    for (let i = 0; i < 5; i++) assert.equal(lane.paint({ label: 'loading', binding: cold, hour: 10 }).surfaces.length, 0);
    release({ projection: { kind: 'cold', family: 'test-lane', project: (view: BoundedReadView) => ({ hour: view.hour, bucket: 0, rows: [], name: '' }) } });
    await settle();
    assert.deepEqual([loads, wakes], [1, 1], 'one import and one wake for the placement');

    const broken = { ...binding(rowsAt([])), kind: 'broken' };
    lane.paint({ label: 'broken', binding: broken, hour: 10 });
    await settle();
    for (let i = 0; i < 5; i++) assert.throws(() => lane.paint({ label: 'broken', binding: broken, hour: 10 }), /chunk missing/);
    assert.equal(loads, 2, 'a failed import is not retried on every paint');
    assert.equal(view.next!({ widget: 1, subs: [], pictures: [], model: broken }, 10, lane.state), null);
    assert.equal(lane.paint({ label: 'retried', binding: broken, hour: 10, generation: 2 }).surfaces.length, 0, 'a re-bind clears the failure and loads again');
    assert.equal(loads, 3);
  } finally {
    recording.restore();
  }
});

registerFamilyProjection<FamilyProjection<Model, Options>>({
  kind: 'unbounded',
  family: 'test-lane',
  project: (view: BoundedReadView): Model => ({ hour: view.hour, bucket: view.hour, rows: [{ key: 'A', v: view.hour }], name: '' }),
});

test('a seek to an hour the held proof cannot locate paints at that hour, through Lane.frame', async () => {
  const flushed = () => new Promise<void>(resolve => setImmediate(resolve));
  const hourly: LaneFamily<Binding, Model, Layout> = { ...family, boundary: (_model, hour) => hour + 1 };
  const recording = installRecordingCanvas();
  try {
    const page = new Page<Bitmap & { content: string }>({ lanes: 1, post: (_lane, m: PageMessage) => { if (m.type === 'frame') lane.frame(m); } });
    const lane: Lane<Bitmap & { content: string }> = new Lane({
      lane: 0, lanes: 1,
      views: () => createLaneView(hourly, () => {}),
      crop: async sources => sources.map(() => ({ content: 'x', width: 1, height: 1, close() {} })),
      present: presentation => page.offer(presentation),
      release: () => {}, fail: () => {}, later: () => {},
    });
    const bound = { ...binding(rowsAt([])), kind: 'unbounded' };
    lane.mount(1, [], 'chart', bound);
    page.mount(1, { route: 1, lane: 0, offset: 0, pin: null, surfaces: [{ install: () => {}, clear: () => {} }] });
    lane.bind(1, { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 0, c: Infinity } });
    page.publish(100, { playing: false, seek: true });
    await flushed(); await flushed(); page.frame(); await flushed(); await flushed();
    assert.deepEqual([page.view(1).mode, page.view(1).painted, page.view(1).fields?.mirror], ['picture', 100, ['A 100']], 'the first reply paints the target, not the fourth hour walked from the window start');
  } finally {
    recording.restore();
  }
});

test('next() builds no identity index, and one snapshot is indexed once for every placement in the worker', () => {
  const recording = installRecordingCanvas();
  const original = IdentityIndex.fromSnapshot;
  let built = 0;
  IdentityIndex.fromSnapshot = snapshot => { built++; return original.call(IdentityIndex, snapshot); };
  try {
    const snapshot: IdentitySnapshot = { scope: { sourceId: 's', revision: 'r', branchId: 'b', content: 1 } };
    const bound = { ...binding(rowsAt([{ key: 'A', v: 1 }])), identitySnapshot: snapshot };
    const views = [createLaneView(family, () => {}), createLaneView(family, () => {})];
    const lanes = views.map(view => driveLane(view, recording.canvases));
    for (const lane of lanes) lane.paint({ label: 'placed', binding: bound, hour: 12 });
    assert.equal(built, 1, 'two placements share one index for one snapshot');
    for (let hour = 12; hour < 400; hour++) for (const [at, view] of views.entries()) view.next!({ widget: 1, subs: [], pictures: [], model: bound }, hour, lanes[at]!.state);
    assert.equal(built, 1, 'next() never indexes a snapshot');
  } finally {
    IdentityIndex.fromSnapshot = original;
    recording.restore();
  }
});

test('the previous layout is given only while the same placement plays forward, never across a seek or a pause', () => {
  const recording = installRecordingCanvas();
  try {
    const seen: (number | null)[] = [];
    const remembering: LaneFamily<Binding, Model, Layout> = {
      ...family,
      layout(model, binding, surface, theme, measure, hour, step) {
        seen.push(step.previous ? step.previous.hour : null);
        return family.layout(model, binding, surface, theme, measure, hour, step);
      },
    };
    const lane = driveLane(createLaneView(remembering, () => {}), recording.canvases);
    lane.paint({ label: 'first', binding: binding(rowsAt([{ key: 'A', v: 1 }])), hour: 10, speed: 24 });
    lane.paint({ label: 'forward', binding: binding(rowsAt([{ key: 'A', v: 2 }])), hour: 12, speed: 24 });
    lane.paint({ label: 'seek back', binding: binding(rowsAt([{ key: 'A', v: 3 }])), hour: 5, speed: 24 });
    lane.paint({ label: 'paused, same hour', binding: binding(rowsAt([{ key: 'A', v: 4 }])), hour: 5, speed: 0 });
    lane.paint({ label: 'paused jump', binding: binding(rowsAt([{ key: 'A', v: 5 }])), hour: 9, speed: 0 });
    assert.deepEqual(seen, [null, 10, null, null, null], 'only continuous forward playback supplies it');
  } finally {
    recording.restore();
  }
});

test('a drawing that expires at a wall-clock time arms the one timer, even while paused, and redraws when it passes', () => {
  const recording = installRecordingCanvas();
  const set: number[] = [];
  const timers = { set: (_fn: () => void, ms: number) => { set.push(ms); return set.length as unknown as ReturnType<typeof setTimeout>; }, clear: () => {} };
  try {
    let draws = 0;
    const holding: LaneFamily<Binding, Model, Layout> = { ...family, paint: (ctx, frame) => { draws++; family.paint(ctx, frame); return frame.now + 500; } };
    const lane = driveLane(createLaneView(holding, () => {}, { timers }), recording.canvases);
    const bound = binding(rowsAt([{ key: 'A', v: 1 }]));
    lane.paint({ label: 'held', binding: bound, hour: 12, speed: 0 });
    assert.deepEqual([draws, set], [1, [500]], 'one timer for the expiry while paused');
    lane.paint({ label: 'still held', binding: bound, hour: 12, speed: 0 });
    assert.deepEqual([draws, set.length], [1, 1], 'no redraw and no second timer before it passes');
    lane.paint({ label: 'expired', binding: bound, hour: 12, speed: 0, now: 1600 });
    assert.equal(draws, 2, 'the expired drawing is redrawn');
  } finally {
    recording.restore();
  }
});
