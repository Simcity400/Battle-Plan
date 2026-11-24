import assert from 'node:assert/strict';
import test from 'node:test';
import type { BoundedReadView } from './contract.ts';
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

registerFamilyProjection<Model, Options>({
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
const binding = (staticPanels: Readonly<Record<string, unknown>>, options: Options = DOWN, extra: Partial<Binding> = {}): Binding => ({
  family: 'test-lane', kind: 'fixture', options, sources: {}, staticPanels, scrollTop: 0, theme: THEME, ...extra,
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
    for (let hour = 0; hour < 1000; hour++) assert.equal(view.next!(scene, hour, lane.state), Infinity, 'loading answers no boundary, so the lane walks no history');
    for (let i = 0; i < 5; i++) assert.equal(lane.paint({ label: 'loading', binding: cold, hour: 10 }).surfaces.length, 0);
    release({ projection: { kind: 'cold', family: 'test-lane', project: (view: BoundedReadView) => ({ hour: view.hour, bucket: 0, rows: [], name: '' }) } });
    await settle();
    assert.deepEqual([loads, wakes], [1, 1], 'one import and one wake for the placement');

    const broken = { ...binding(rowsAt([])), kind: 'broken' };
    lane.paint({ label: 'broken', binding: broken, hour: 10 });
    await settle();
    for (let i = 0; i < 5; i++) assert.throws(() => lane.paint({ label: 'broken', binding: broken, hour: 10 }), /chunk missing/);
    assert.equal(loads, 2, 'a failed import is not retried on every paint');
    assert.equal(view.next!({ widget: 1, subs: [], pictures: [], model: broken }, 10, lane.state), Infinity);
    assert.equal(lane.paint({ label: 'retried', binding: broken, hour: 10, generation: 2 }).surfaces.length, 0, 'a re-bind clears the failure and loads again');
    assert.equal(loads, 3);
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
