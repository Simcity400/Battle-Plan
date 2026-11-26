import assert from 'node:assert/strict';
import test from 'node:test';
import { Page, type Bitmap, type PageMessage } from '../dashboard/follow/page.ts';
import { Lane } from '../draw/lane.ts';
import { cropSurfaces } from '../draw/laneDispatch.ts';
import type { BoundedReadView, FamilyProjection } from './contract.ts';
import { createLaneView, lazyLaneView, type LaneBinding, type LaneFamily } from './laneRuntime.ts';
import { familyBinding, type FamilyBindingContext, type FamilyHostAdapter } from './host/adapter.ts';
import { assertContinuity, assertFreshPaint, assertQuietFrames, assertTooltipDated, driveLane, installRecordingCanvas, LANE_THEME as THEME, LaneTransport } from './laneRuntime.fixture.ts';
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
  view: [],
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
    return row ? { rowKey: key, content: { title: key, rows: [] } as never } : null;
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
    const scene = { widget: 1, subs: [], pictures: [], model: lane.model() };
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

test('while playing, a correction inside the cadence never publishes the old tooltip or mirror beside the corrected picture', () => {
  reset();
  const recording = installRecordingCanvas();
  try {
    const lane = driveLane(createLaneView(family, () => {}), recording.canvases);
    lane.paint({ label: 'first', binding: binding(rowsAt([{ key: 'A', v: 1 }])), hour: 12, speed: 24, inspect: 'A', seek: 1 });
    const corrected = lane.paint({ label: 'corrected', binding: binding(rowsAt([{ key: 'A', v: 7 }])), hour: 12, speed: 24, inspect: 'A', seek: 1 });
    assert.deepEqual((corrected.fields.tooltip?.value as { text: { rowKey: string } } | undefined)?.text?.rowKey, 'A');
    assert.ok(counts.tooltip >= 2, 'the tooltip was derived again from the corrected frame');
    assert.equal(corrected.fields.mirror, undefined, 'the mirror is withheld until it is due, never the old rows');
    const due = lane.paint({ label: 'paused', binding: binding(rowsAt([{ key: 'A', v: 7 }])), hour: 12, speed: 0, inspect: 'A', seek: 1 });
    assert.deepEqual(due.fields.mirror?.value, ['A 7'], 'paused, the mirror is published at once');
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

test('a secondary panel read on a clock 24 hours ahead reaches the projection with its shift, never as future data', () => {
  reset();
  const recording = installRecordingCanvas();
  try {
    const lane = driveLane(createLaneView(family, () => {}), recording.canvases);
    const painted = lane.paint({ label: 'shifted', hour: 12, binding: { ...binding({ main: { hour: 36, rows: [{ key: 'A', v: 1 }] } }), staticShifts: { main: 24 } } });
    assert.equal(painted.mode, 'picture');
    assert.deepEqual(painted.fields.nav?.value, ['A']);
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

test('a cold projection or lane code load walks no history, keeps one waiter per placement, and a failed import fails by name until the host’s retry re-binds it', async () => {
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

    let releaseLane: (lane: LaneFamily<Binding, Model, Layout>) => void = () => {};
    let laneLoads = 0;
    const laneWakes: number[] = [];
    const lazy = lazyLaneView('test-lane', () => { laneLoads++; return new Promise(resolve => { releaseLane = resolve; }); }, widget => laneWakes.push(widget));
    const lazyLane = driveLane(lazy, recording.canvases);
    const shown = binding(rowsAt([{ key: 'A', v: 1 }]));
    for (const widget of [1, 2]) assert.equal(lazy.next!({ widget, subs: [], pictures: [], model: shown }, 10, lazyLane.state), null, 'loading lane code locates no change');
    for (let i = 0; i < 3; i++) assert.equal(lazyLane.paint({ label: 'lane loading', binding: shown, hour: 10 }).waiting, true, 'a placement waits, as for a loading projection');
    releaseLane(family);
    await settle();
    assert.deepEqual([laneLoads, laneWakes], [1, [1, 2]], 'one import, and each waiting placement woken once');
    assert.ok(lazyLane.paint({ label: 'lane loaded', binding: shown, hour: 10 }).surfaces.length > 0);
    assert.deepEqual(lazyLane.picture(), ['t:0:0=#eef1f6|A 1  1']);

    const failing = lazyLaneView('test-lane', () => { laneLoads++; return Promise.reject(new Error('lane chunk missing')); }, () => {});
    const failed = driveLane(failing, recording.canvases);
    failed.paint({ label: 'lane broken', binding: shown, hour: 10 });
    await settle();
    for (let i = 0; i < 3; i++) assert.throws(() => failed.paint({ label: 'lane broken', binding: shown, hour: 10 }), /The test-lane lane did not load: lane chunk missing/);
    assert.equal(laneLoads, 2, 'a failed lane import is not retried on every paint');
    assert.equal(failed.paint({ label: 'lane retried', binding: shown, hour: 10, generation: 2 }).surfaces.length, 0, 'a re-bind clears the failure and loads the lane code again');
    assert.equal(laneLoads, 3);
  } finally {
    recording.restore();
  }
});

registerFamilyProjection<FamilyProjection<Model, Options>>({
  kind: 'unbounded',
  family: 'test-lane',
  project: (view: BoundedReadView): Model => ({ hour: view.hour, bucket: view.hour, rows: [{ key: 'A', v: view.hour }], name: '' }),
});

test('a placement unmounted while its projection loads is not woken by the load, and a loading paint waits instead of presenting', async () => {
  let release: (value: unknown) => void = () => {};
  bindFamilyManifest([{ family: 'test-lane', kind: 'slow', loadSpec: () => new Promise(resolve => { release = resolve; }) as never }]);
  const recording = installRecordingCanvas();
  try {
    let wakes = 0;
    const view = createLaneView(family, () => { wakes++; });
    const lane = driveLane(view, recording.canvases);
    const slow = { ...binding(rowsAt([])), kind: 'slow' };
    assert.equal(lane.paint({ label: 'loading', binding: slow, hour: 10 }).waiting, true, 'nothing to present until the import settles');
    view.unmount!(1, lane.state);
    release({ projection: { kind: 'slow', family: 'test-lane', project: (read: BoundedReadView) => ({ hour: read.hour, bucket: 0, rows: [], name: '' }) } });
    await settle();
    assert.equal(wakes, 0, 'the unmounted placement left the registry');
  } finally {
    recording.restore();
  }
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

test('across the worker boundary a scroll, a focus change and an art arrival cost no projection, layout or index, and two placements share one index', () => {
  reset();
  const recording = installRecordingCanvas();
  const original = IdentityIndex.fromSnapshot;
  let built = 0;
  IdentityIndex.fromSnapshot = snapshot => { built++; return original.call(IdentityIndex, snapshot); };
  try {
    const adapter: FamilyHostAdapter<Model, Options, Binding> = { family: 'test-lane', navigation: 'rows' };
    const snapshot: IdentitySnapshot = { scope: { sourceId: 's', revision: 'r', branchId: 'b', content: 1 } };
    const options: Options = { order: 'down', name: 'GER' };
    const sources = {};
    const staticPanels = rowsAt([{ key: 'A', v: 1 }, { key: 'B', v: 2 }]);
    const viewport = { width: 320, height: 200, dpr: 1, scrollLeft: 0, scrollTop: 0, zoomWindow: null };
    const context = (change: Partial<FamilyBindingContext<Options>> = {}): FamilyBindingContext<Options> => ({
      kind: 'fixture', view: 'default', options, settings: undefined, sources, extent: null, plan: null, viewport, theme: THEME, reducedMotion: false,
      selectedKey: null, focusedKey: null, expanded: {}, audit: false, auditId: undefined, pauseFlush: 0, ...change,
    });
    const bound = (change?: Partial<FamilyBindingContext<Options>>): Binding => ({ ...familyBinding(adapter, context(change))!, staticPanels, identitySnapshot: snapshot });
    const transport = new LaneTransport();
    const views = [createLaneView(family, () => {}), createLaneView(family, () => {})];
    const lanes = views.map((view, at) => driveLane(view, recording.canvases, undefined, transport, `placement ${at}`));
    const paintBoth = (binding: Binding) => { for (const lane of lanes) lane.paint({ label: 'bind', binding, hour: 12 }); };
    paintBoth(bound());
    assert.deepEqual([counts.project, counts.layout, built], [2, 2, 1], 'two placements of one snapshot share one index, though each was cloned');
    const sent = transport.sent.payloads;
    const steps: [string, Binding, number][] = [
      ['a scroll', bound({ viewport: { ...viewport, scrollTop: 40 } }), 0],
      ['a focus change', bound({ viewport: { ...viewport, scrollTop: 40 }, focusedKey: 'B' }), 0],
      ['an art arrival', { ...bound({ viewport: { ...viewport, scrollTop: 40 }, focusedKey: 'B' }), artResolutions: { 'sprite:x': { status: 'noArtExpected' } } }, 1],
    ];
    let payloads = sent;
    for (const [label, binding, crossed] of steps) {
      reset();
      paintBoth(binding);
      assert.deepEqual([counts.project, counts.layout, built], [0, 0, 1], `${label}: no projection, no layout, no index build`);
      assert.equal(transport.sent.payloads - payloads, crossed, `${label}: crosses only what changed`);
      payloads = transport.sent.payloads;
      assert.equal(counts.paint, 2, `${label}: each placement redraws`);
    }
    for (let hour = 12; hour < 400; hour++) for (const [at, view] of views.entries()) view.next!({ widget: 1, subs: [], pictures: [], model: lanes[at]!.model() }, hour, lanes[at]!.state);
    assert.equal(built, 1, 'next() never indexes a snapshot');
  } finally {
    IdentityIndex.fromSnapshot = original;
    recording.restore();
  }
});

test('a changed static answer, option or source mapping crosses once and rebuilds; an unchanged one keeps its revision', () => {
  reset();
  const recording = installRecordingCanvas();
  try {
    const transport = new LaneTransport();
    const lane = driveLane(createLaneView(family, () => {}), recording.canvases, undefined, transport);
    const a = { rows: [{ key: 'A', v: 1 }] };
    const b = { rows: [{ key: 'B', v: 2 }] };
    const sources = { main: 3 };
    const bound = (panels: Readonly<Record<string, unknown>>, options: Options = DOWN, mapping: Readonly<Record<string, number>> = sources) =>
      ({ ...binding(panels, options), sources: mapping });
    lane.paint({ label: 'first', binding: bound({ main: a, other: b }), hour: 12 });
    const after = (label: string, next: Binding, projected: number, crossed: number) => {
      reset();
      const sent = transport.sent.payloads;
      lane.paint({ label, binding: next, hour: 12 });
      assert.deepEqual([counts.project, transport.sent.payloads - sent], [projected, crossed], label);
    };
    after('a new bundle with the same answers', bound({ main: a, other: b }), 0, 0);
    after('one changed answer', bound({ main: { rows: [{ key: 'A', v: 9 }] }, other: b }), 1, 1);
    const up: Options = { order: 'up' };
    after('another option', bound({ main: a, other: b }, up), 1, 2);
    after('another source mapping', bound({ main: a, other: b }, up, { main: 4 }), 1, 1);
  } finally {
    recording.restore();
  }
});

registerFamilyProjection<FamilyProjection<Model, Options>>({
  kind: 'windowed',
  family: 'test-lane',
  project(view: BoundedReadView): Model {
    counts.project++;
    const first = view.rowWindow?.first ?? 0;
    return { hour: view.hour, bucket: 0, rows: [{ key: `row ${first}`, v: first }], name: '' };
  },
});

test('a row window change re-projects only a projection that read the window', () => {
  const recording = installRecordingCanvas();
  try {
    const lane = driveLane(createLaneView(family, () => {}), recording.canvases);
    const rows = rowsAt([{ key: 'A', v: 1 }]);
    const at = (first: number) => ({ first, count: 10 });
    lane.paint({ label: 'plain', binding: binding(rows, DOWN, { window: at(0) }), hour: 12 });
    reset();
    lane.paint({ label: 'plain, scrolled', binding: binding(rows, DOWN, { window: at(40) }), hour: 12 });
    assert.equal(counts.project, 0, 'a projection that never read the window is reused');
    const windowed = (first: number) => ({ ...binding(rows, DOWN, { window: at(first) }), kind: 'windowed' });
    lane.paint({ label: 'windowed', binding: windowed(0), hour: 12 });
    reset();
    lane.paint({ label: 'windowed, same window', binding: windowed(0), hour: 12 });
    assert.equal(counts.project, 0);
    lane.paint({ label: 'windowed, scrolled', binding: windowed(40), hour: 12 });
    assert.equal(counts.project, 1, 'a projection that read the window projects the new one');
    assert.deepEqual(lane.picture(), ['t:0:0=#eef1f6|row 40 40  0']);
  } finally {
    recording.restore();
  }
});

const GLIDE_MS = 100;
const gliding: LaneFamily<Binding, Model, Layout> = {
  ...family,
  status: model => (model.rows.length ? 'picture' : { refused: 'Nothing was recorded at this hour' }),
  paint(ctx, frame) {
    ctx.clearRect(0, 0, frame.surface.width, frame.surface.height);
    ctx.fillStyle = frame.theme.text;
    const moving = frame.before !== null && frame.now - frame.changedAt < GLIDE_MS;
    const from = new Map(frame.before?.layout.rows.map(row => [row.key, row.y]) ?? []);
    for (const row of frame.layout.rows) ctx.fillText(row.key, 0, moving ? (from.get(row.key) ?? row.y) : row.y);
    return moving;
  },
};

test('a tooltip on a picture that stands across hours is dated by the hour it is shown at, as a fresh paint dates it', () => {
  assertTooltipDated(() => createLaneView(family, () => {}), { label: 'fixture', binding: binding(rowsAt([{ key: 'A', v: 1 }])), hour: 12 }, 'A');
});

test('a transition continues only from the last drawn placement: a seek, a pin or a refusal between drops it even when the model is reused, and a same-hour tick keeps it', () => {
  assertContinuity(() => createLaneView(gliding, () => {}), {
    from: { label: 'before the swap', binding: binding(rowsAt([{ key: 'A', v: 1 }, { key: 'B', v: 2 }])), hour: 11 },
    refused: { label: 'an unrecorded hour', binding: binding(rowsAt([])), hour: 12 },
    to: { label: 'the swap', binding: binding(rowsAt([{ key: 'B', v: 2 }, { key: 'A', v: 1 }])), hour: 13 },
  });
});

test('a refused frame leaves no layout for the next frame to continue from', () => {
  const recording = installRecordingCanvas();
  try {
    const seen: (number | null)[] = [];
    const remembering: LaneFamily<Binding, Model, Layout> = {
      ...gliding,
      layout(model, binding, surface, theme, measure, hour, step) {
        seen.push(step.previous ? step.previous.hour : null);
        return gliding.layout(model, binding, surface, theme, measure, hour, step);
      },
    };
    const lane = driveLane(createLaneView(remembering, () => {}), recording.canvases);
    lane.paint({ label: 'drawn', binding: binding(rowsAt([{ key: 'A', v: 1 }])), hour: 10, speed: 24 });
    assert.equal(lane.paint({ label: 'refused', binding: binding(rowsAt([])), hour: 11, speed: 24 }).mode, 'refused');
    lane.paint({ label: 'drawn again', binding: binding(rowsAt([{ key: 'A', v: 2 }])), hour: 12, speed: 24 });
    assert.deepEqual(seen, [null, null], 'the frame after a refusal continues from nothing');
  } finally {
    recording.restore();
  }
});

test('a family without a mirror arms no mirror timer when its model changes while playing', () => {
  const set: number[] = [];
  const timers = { set: (_fn: () => void, ms: number) => { set.push(ms); return set.length as unknown as ReturnType<typeof setTimeout>; }, clear: () => {} };
  const recording = installRecordingCanvas();
  try {
    const silent: LaneFamily<Binding, Model, Layout> = { ...family, mirror: null };
    const lane = driveLane(createLaneView(silent, () => {}, { timers }), recording.canvases);
    for (let hour = 12; hour < 60; hour += 10) {
      const painted = lane.paint({ label: `hour ${hour}`, binding: binding(rowsAt([{ key: 'A', v: hour }])), hour, speed: 24, now: 1000 + hour });
      assert.equal(painted.fields.mirror, undefined);
    }
    assert.deepEqual(set, [], 'no timer for a field the family never publishes');
  } finally {
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

test('incremental families keep their backing canvas out of destructive transferToImageBitmap', async () => {
  const recording = installRecordingCanvas();
  try {
    const incremental: LaneFamily<Binding, Model, Layout> = { ...family, paint: () => ({ drew: true, next: false }) };
    const view = createLaneView(incremental, () => {});
    const lane = driveLane(view, recording.canvases);
    const painted = lane.paint({ label: 'inc', binding: binding(rowsAt([{ key: 'A', v: 1 }])), hour: 10 });
    const canvas = painted.surfaces[0]!.source as unknown as { width: number; height: number; transferToImageBitmap?: () => unknown };
    let transfers = 0;
    canvas.transferToImageBitmap = () => { transfers++; throw new Error('must not transfer incremental canvas'); };
    let fallbacks = 0;
    const bitmaps = await cropSurfaces([canvas], async source => {
      fallbacks++;
      return { width: source.width, close() {} };
    });
    assert.equal(bitmaps.length, 1);
    assert.equal(transfers, 0, 'incremental family canvas is never detached');
    assert.equal(fallbacks, 1, 'incremental family uses non-destructive fallback snapshot');
  } finally {
    recording.restore();
  }
});

test('a zero-sized surface waits without presenting or looping kept-content refusals until resized', async () => {
  const recording = installRecordingCanvas();
  try {
    let presentations = 0;
    let refusals = 0;
    const view = createLaneView(family, () => {});
    let lane!: Lane<Bitmap>;
    const page = new Page<Bitmap>({
      lanes: 1,
      post: (_lane, m: PageMessage) => {
        if (m.type === 'frame') {
          if (m.refused?.length) refusals += m.refused.length;
          lane.frame(m);
        }
      },
    });
    lane = new Lane<Bitmap>({
      lane: 0,
      lanes: 1,
      clock: () => 0,
      views: () => view,
      crop: async sources => sources.map(s => ({ width: s.width, close() {} })),
      present: presentation => { presentations++; page.offer(presentation); },
      release: () => {},
      fail: reason => { throw new Error(reason); },
      later: () => {},
    });
    const bound = binding(rowsAt([{ key: 'A', v: 1 }]));
    lane.mount(1, [], 'test-lane', bound);
    lane.bind(1, { generation: 1, owner: 'o1', surfaces: [{ width: 0, height: 0, dpr: 1 }], offset: 0, pin: null, dated: { h: 0, c: 100 } });
    page.mount(1, { route: 0, lane: 0, owner: 'o1', surfaces: [{ install() {}, clear() {} }] });
    for (let step = 0; step < 4; step++) {
      page.publish(10, { playing: false, seek: 0 });
      await new Promise<void>(r => setImmediate(r));
      page.frame();
    }
    assert.equal(presentations, 0, 'zero-sized surface never presents');
    assert.equal(refusals, 0, 'zero-sized surface never loops kept-content refusals');
    lane.bind(1, { generation: 2, owner: 'o1', surfaces: [{ width: 200, height: 100, dpr: 1 }], offset: 0, pin: null, dated: { h: 0, c: 100 } });
    page.rebind(1, { owner: 'o1' });
    page.publish(10, { playing: false, seek: 0 });
    await new Promise<void>(r => setImmediate(r));
    page.frame();
    assert.equal(page.view(1).mode, 'picture', 'resizing to positive dimensions wakes the view and installs');
  } finally {
    recording.restore();
  }
});

test('non-hover frames deduplicate structure, mirror, and audit fields structurally without JSON.stringify', () => {
  const recording = installRecordingCanvas();
  const origStringify = JSON.stringify;
  let stringifyCalls = 0;
  try {
    const lane = driveLane(createLaneView({
      ...family,
      paint: () => false,
      structure: frame => ({ nav: frame.model.rows.map(r => [r.key, BigInt(r.v)]), meta: { bucket: frame.model.bucket } }),
      audit: frame => frame.model.rows.map(r => ({ key: r.key, value: BigInt(r.v) })),
    }, () => {}), recording.canvases);
    JSON.stringify = ((...args: Parameters<typeof JSON.stringify>) => {
      stringifyCalls++;
      return origStringify(...args);
    }) as typeof JSON.stringify;
    const first = lane.paint({ label: 'p1', hour: 10, binding: { ...binding(rowsAt([{ key: 'A', v: 1 }])), audit: true } });
    const same = lane.paint({ label: 'p2', hour: 12, binding: { ...binding(rowsAt([{ key: 'A', v: 1 }])), audit: true } });
    const changed = lane.paint({ label: 'p3', hour: 25, binding: { ...binding(rowsAt([{ key: 'A', v: 2 }])), audit: true } });
    assert.equal(first.fields.nav!.content, same.fields.nav!.content, 'identical nav keeps the same content token');
    assert.equal(first.fields.audit!.content, same.fields.audit!.content, 'identical audit keeps the same content token');
    assert.notEqual(first.fields.nav!.content, changed.fields.nav!.content, 'changed bigint nav produces a new content token');
    assert.notEqual(first.fields.audit!.content, changed.fields.audit!.content, 'changed bigint audit produces a new content token');
    assert.equal(stringifyCalls, 0, 'field deduplication never calls JSON.stringify');
  } finally {
    JSON.stringify = origStringify;
    recording.restore();
  }
});
