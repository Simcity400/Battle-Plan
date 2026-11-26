import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Decoder, encode } from '@msgpack/msgpack';
import { Lane, type LaneHost, type LaneView, type Painted, type Presented } from './lane.ts';
import { cropAll, cropSurfaces, macrotaskSchedule, markCanvasPainted, type WakeChannel } from './laneDispatch.ts';
import { startLaneWorker } from './lane.worker.ts';
import type { FromLane, FromPull, ToLane } from './laneMessages';
import { fieldsView } from './views/fields.ts';
import { Page, type Bitmap, type FrameRequest, type PageMessage } from '../dashboard/follow/page.ts';
import { message } from '../dashboard/follow/messages.fixture.ts';
import { parseAnswer, cellValue } from '../dashboard/follow/message.ts';
import { splitForLanes } from '../dashboard/follow/split.ts';
import { componentReading } from '../dashboard/follow/useFollowed.ts';
import { installedEntry } from '../dashboard/follow/store.ts';
import type { Pipeline } from '../dashboard/follow/pipeline';
import { PayloadSender, PayloadStore } from '../dashboard/follow/payloads.ts';
import { boundDated } from '../families/host/dated.ts';

interface Fake extends Bitmap { content: string; closed: boolean }
const NATIVE = 9, ROUTE = 1;
const flush = () => new Promise<void>(resolve => setImmediate(resolve));

type PanelRow = { id: number; hour: number; kind: 'data' | 'stands' | 'waiting' | 'completed'; data?: unknown; stepped?: boolean };
function panels(records: PanelRow[], seek = 0): Uint8Array {
  const parts = records.map(record => {
    const body = record.kind === 'data' ? encode(record.data) : new Uint8Array();
    const out = new Uint8Array((16 + body.length + 7) & ~7), view = new DataView(out.buffer);
    view.setUint32(0, record.kind === 'completed' ? 0 : record.id, true); view.setInt32(4, record.hour, true);
    view.setUint8(8, { data: 1, stands: 3, completed: 4, waiting: 5 }[record.kind]); view.setUint8(9, record.stepped ? 1 : 0);
    view.setUint32(12, body.length, true); out.set(body, 16);
    return out;
  });
  const out = new Uint8Array(24 + parts.reduce((sum, part) => sum + part.length, 0)), view = new DataView(out.buffer);
  view.setUint32(0, 0x324c4e50, true); view.setUint16(4, 6, true); view.setUint32(8, records.length, true); view.setUint32(16, NATIVE, true); view.setUint32(20, seek, true);
  let at = 24;
  for (const part of parts) { out.set(part, at); at += part.length; }
  return out;
}

interface Timing { offset?: number; pin?: number | null; dated?: { h: number; c: number } }

function rig(views: Record<string, LaneView> = {}, extra: { budget?: number; crop?: LaneHost<Fake>['crop']; clock?: () => number; earlyBuffer?: number } = {}) {
  const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => { posted.push(m); if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target); } });
  const posted: PageMessage[] = [], released: number[] = [], failures: string[] = [], later: (() => void)[] = [], hits: (string | null)[] = [];
  const subFailures: [number, string][] = [], widgetFailures: [number, string][] = [];
  const widgetSubs = new Map<number, number[]>();
  const presented: Presented<Fake>[] = [];
  const shown = new Map<number, (string | null)[]>();
  const lane: Lane<Fake> = new Lane<Fake>({
    lane: 0, lanes: 1,
    views: kind => views[kind] ?? fieldsView(() => (panel => (panel.data as { shape?: string } | undefined)?.shape)),
    crop: async sources => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: 1, close() { this.closed = true; } })),
    ...extra,
    present: presentation => { presented.push(presentation); page.offer(presentation); },
    release: bytes => released.push(bytes),
    fail: reason => failures.push(reason),
    failSub: (sub, reason) => { subFailures.push([sub, reason]); for (const [w, list] of widgetSubs) if (list.includes(sub)) page.name(w, 'failed', reason); },
    failWidget: (widget, reason) => { widgetFailures.push([widget, reason]); page.name(widget, 'failed', reason); },
    later: run => later.push(run),
    hit: (_widget, key) => hits.push(key),
  });
  const mount = (widget: number, subs: { id: number; channel: 'panel' | 'followed'; product?: string }[], view = 'fields', surfaces = 0, timing: Timing = {}) => {
    for (const sub of subs) lane.follow(sub.id, sub.channel, sub.product);
    widgetSubs.set(widget, subs.map(sub => sub.id));
    lane.mount(widget, subs.map(sub => sub.id), view);
    const list: (string | null)[] = [];
    shown.set(widget, list);
    page.mount(widget, { route: ROUTE, lane: 0, offset: timing.offset ?? 0, pin: timing.pin ?? null,
      surfaces: Array.from({ length: surfaces }, (_, k) => ({ install: (b: Fake) => { list[k] = b.content; }, clear: () => { list[k] = null; } })) });
    lane.bind(widget, { generation: 1, surfaces: Array.from({ length: surfaces }, () => ({ width: 1, height: 1, dpr: 1 })), offset: timing.offset ?? 0, pin: timing.pin ?? null,
      ...timing.dated ? { dated: timing.dated } : {} });
  };
  const step = async (T: number, options: Parameters<Page<Fake>['publish']>[1] = {}) => { page.publish(T, options); await flush(); await flush(); page.frame(); await flush(); };
  return { page, lane, posted, released, failures, subFailures, widgetFailures, later, hits, presented, shown, mount, step, frames: () => posted.filter((m): m is FrameRequest => m.type === 'frame') };
}

const rows = (keys: bigint[], values: bigint[], extra: { changed?: boolean } = {}) => ({ keys, columns: 1, values, ...extra });
const valueOf = (view: { fields?: Readonly<Record<string, unknown>> }, sub: number, row = 0) => {
  const answer = parseAnswer(view.fields?.[`answer:${sub}`] as Uint8Array);
  return answer.kind === 'notRecorded' ? null : cellValue(answer, row);
};
function withLimits(bytes: Uint8Array, limits: [id: number, through: number][]): Uint8Array {
  const out = new Uint8Array(bytes.length + 16 * limits.length), view = new DataView(out.buffer);
  out.set(bytes);
  view.setUint32(12, view.getUint32(12, true) + limits.length, true);
  limits.forEach(([id, through], k) => { view.setUint32(bytes.length + 16 * k, id, true); view.setInt32(bytes.length + 16 * k + 4, through, true); });
  return out;
}

test('a panel installs its model and live values whole, dated from the actual target', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: { shape: 'A', n: 1 } }, { id: 0, hour: 20, kind: 'completed' }]));
  await step(15, { playing: false });
  const view = page.view(1);
  assert.deepEqual([view.mode, view.h, view.through, (view.fields?.['model:3'] as { structure?: string }).structure], ['picture', 10, 15, 'A']);
  lane.receive('panels', panels([{ id: 3, hour: 16, kind: 'data', stepped: true, data: { shape: 'A', n: 2 } }, { id: 0, hour: 20, kind: 'completed' }]));
  await step(17);
  assert.deepEqual([page.view(1).h, (page.view(1).fields?.['model:3'] as { structure?: string }).structure], [16, 'A'], 'an equal structure keeps the installed model; its date moves on');
});

test('identical panel bytes decode once within the subscription and seek', t => {
  const { lane, mount } = rig();
  const reads = t.mock.method(Decoder.prototype, 'decode');
  const send = (id: number, hour: number, n: number, seek = 0) =>
    lane.receive('panels', panels([{ id, hour, kind: 'data', data: { n } }], seek));
  mount(1, [{ id: 3, channel: 'panel' }, { id: 4, channel: 'panel' }]);
  send(3, 10, 1);
  send(3, 11, 1);
  lane.bindRoute(3, ROUTE, NATIVE);
  send(3, 12, 1);
  assert.equal(reads.mock.callCount(), 1);
  const first = lane.facts.latest(3);
  lane.bindRoute(4, ROUTE, NATIVE);
  send(4, 12, 1);
  const other = lane.facts.latest(4);
  assert.ok(first?.kind === 'panel' && other?.kind === 'panel');
  assert.notEqual(first.data, other.data);
  send(3, 13, 2);
  lane.stream(ROUTE, 1, 13);
  send(3, 13, 2, 1);
  assert.equal(reads.mock.callCount(), 4);
});

test('records that arrive before their route is known are held and land once it is', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', data: { shape: 'S' } }]));
  await step(10, { playing: false });
  assert.equal(page.view(1).mode, 'reading');
  lane.bindRoute(3, ROUTE, NATIVE);
  await step(10);
  assert.equal(page.view(1).h, 10);
});

test('changed rows merge onto the followed table and their bytes come back once the target passes them', async () => {
  const { page, lane, released, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  const whole = message(10, [{ id: 5, answer: { keys: [1n, 2n], columns: 1, values: [10n, 20n] } }], false, NATIVE);
  lane.receive('follow', whole);
  lane.receive('follow', message(20, [{ id: 5, answer: { keys: [2n], columns: 1, values: [25n], changed: true } }], false, NATIVE));
  await step(20, { playing: false });
  const body = page.view(1).fields?.['answer:5'] as Uint8Array;
  const answer = parseAnswer(body);
  assert.ok(answer.kind === 'rows');
  assert.deepEqual([cellValue(answer, 0), cellValue(answer, 1), page.view(1).h], [10n, 25n, 20]);
  const before = released.reduce((sum, bytes) => sum + bytes, 0);
  lane.receive('follow', message(30, [{ id: 5, answer: { keys: [1n], columns: 1, values: [11n], changed: true } }], false, NATIVE));
  await step(30);
  await step(31);
  assert.ok(released.reduce((sum, bytes) => sum + bytes, 0) > before + whole.length - 24 - 16, 'the facts behind the installed picture are released');
});

test('addendum 1: the painter paints at the window start, which breaks at the change hours', async () => {
  const hours: number[] = [];
  const probe: LaneView = {
    next: (_scene, hour) => (Math.floor(hour / 24) + 1) * 24,
    paint: (input): Painted => { hours.push(input.hour); return { surfaces: [], fields: { day: { value: Math.floor(input.hour / 24), content: `day:${input.hour}` } }, motion: false }; },
  };
  const { page, lane, mount, step } = rig({ probe });
  mount(1, [{ id: 3, channel: 'panel' }], 'probe');
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 96, kind: 'data', stepped: true, data: {} }, { id: 0, hour: 140, kind: 'completed' }]));
  await step(124, { playing: false });
  assert.deepEqual([page.view(1).h, page.view(1).through, page.view(1).fields?.day], [96, 124, 5], 'the stepped panel of 96 shows the day of 124');
  await step(130);
  assert.deepEqual(hours, [120], 'the same day keeps its picture');
});

test('the lane sends a bounded picture while the page can still install it', async () => {
  const probe: LaneView = {
    paint: ({ hour }) => ({ surfaces: [], fields: { first: { value: hour, content: String(hour) } }, nextChangeAt: hour + 4, motion: false }),
  };
  const { page, lane, mount, step, presented } = rig({ probe });
  mount(1, [{ id: 3, channel: 'panel' }], 'probe', 0, { offset: 10 });
  lane.bindRoute(3, ROUTE, NATIVE);
  const supply = (hour: number) => lane.receive('panels', panels([{ id: 3, hour, kind: 'data', stepped: true, data: { hour } }, { id: 0, hour: 100, kind: 'completed' }]));
  supply(10);
  await step(3, { playing: true, step: 10 });
  assert.equal(page.view(1).fields?.first, 10);
  presented.length = 0;
  supply(14);
  await step(8, { step: 10 });
  assert.deepEqual(presented.flatMap(p => p.entries), []);
  assert.equal(page.view(1).fields?.first, 10);
  supply(19);
  await step(10, { step: 10 });
  assert.equal(page.view(1).fields?.first, 19);
});

test('playing past its confirmed hour, a time-dependent widget shows its newest known picture instead of keeping the first one', async () => {
  const probe: LaneView = {
    paint: ({ hour }) => ({ surfaces: [], fields: { first: { value: hour, content: String(hour) } }, nextChangeAt: hour + 1, motion: false }),
  };
  const { page, lane, mount, step } = rig({ probe });
  mount(1, [{ id: 3, channel: 'panel' }], 'probe');
  lane.bindRoute(3, ROUTE, NATIVE);
  const supply = (hour: number) => lane.receive('panels', panels([{ id: 3, hour, kind: 'data', stepped: true, data: { hour } }]));
  supply(10);
  await step(10, { playing: false });
  assert.equal(page.view(1).fields?.first, 10);
  await step(10, { playing: true, speed: 1440, step: 24 });
  supply(14);
  await step(40);
  assert.equal(page.view(1).fields?.first, 14, 'the picture of the newest known hour replaces the first one');
  supply(30);
  await step(64);
  assert.equal(page.view(1).fields?.first, 30);
  await step(88);
  assert.equal(page.view(1).fields?.first, 30, 'with no newer data the newest known picture stays');
});

test('a statically dated time chart repaints at the newest hour its supply covers while playing, though its next change lies past that hour', async () => {
  const probe: LaneView = {
    next: (_scene, hour) => hour + 1,
    paint: ({ hour }) => ({ surfaces: [], fields: { first: { value: hour, content: String(hour) } }, nextChangeAt: hour + 1, motion: false }),
  };
  const { page, lane, mount, step } = rig({ probe });
  mount(1, [], 'probe', 0, { dated: { h: 0, c: 20 } });
  await step(20, { playing: false });
  assert.equal(page.view(1).fields?.first, 20);
  await step(20, { playing: true, speed: 1440, step: 24 });
  lane.supply(1, { dated: { h: 0, c: 40 } });
  await step(60);
  assert.equal(page.view(1).fields?.first, 40, 'the supply now covers 40, so its picture shows though the playhead is past it');
});

test('paused past what its supply covers, a statically dated time chart mounted fresh still waits for a covering picture', async () => {
  const probe: LaneView = {
    next: (_scene, hour) => hour + 1,
    paint: ({ hour }) => ({ surfaces: [], fields: { first: { value: hour, content: String(hour) } }, nextChangeAt: hour + 1, motion: false }),
  };
  const { page, mount, step } = rig({ probe });
  mount(1, [], 'probe', 0, { dated: { h: 0, c: 40 } });
  await step(60, { playing: false });
  assert.equal(page.view(1).mode, 'reading');
});

test('playing past its confirmed hour, a newer picture with the same day deadline as the shown one still replaces it', async () => {
  const probe: LaneView = {
    paint: ({ hour }) => ({ surfaces: [], fields: { first: { value: hour, content: String(hour) } }, nextChangeAt: (Math.floor(hour / 24) + 1) * 24, motion: false }),
  };
  const { page, lane, mount, step } = rig({ probe });
  mount(1, [{ id: 3, channel: 'panel' }], 'probe');
  lane.bindRoute(3, ROUTE, NATIVE);
  const supply = (hour: number) => lane.receive('panels', panels([{ id: 3, hour, kind: 'data', stepped: true, data: { hour } }]));
  supply(10);
  await step(10, { playing: false });
  await step(10, { playing: true, speed: 1440, step: 24 });
  supply(14);
  await step(40);
  assert.equal(page.view(1).fields?.first, 14, 'the picture of 14 replaces the picture of 10 though both were painted with the same day deadline');
});

test('after a backward seek while playing, a picture from before the seek never holds back the newest known one', async () => {
  const probe: LaneView = {
    paint: ({ hour }) => ({ surfaces: [], fields: { first: { value: hour, content: String(hour) } }, nextChangeAt: hour + 1, motion: false }),
  };
  const { page, lane, mount, step } = rig({ probe });
  mount(1, [{ id: 3, channel: 'panel' }], 'probe');
  lane.bindRoute(3, ROUTE, NATIVE);
  const supply = (hour: number, seek = 0) => lane.receive('panels', panels([{ id: 3, hour, kind: 'data', stepped: true, data: { hour } }], seek));
  supply(100);
  await step(100, { playing: false });
  await step(130, { playing: true, speed: 1440, step: 24 });
  assert.equal(page.view(1).fields?.first, 100);
  supply(30, 1);
  await step(40, { seek: 1 });
  await step(64);
  assert.equal(page.view(1).fields?.first, 30, 'the picture of 30 shows though the lane last installed 100 before the seek');
});

test('addendum 2: a moving widget stays due, one step per acknowledgement, after every widget whose data changed', async () => {
  let frames = 0;
  const moving: LaneView = { paint: (): Painted => ({ surfaces: [], fields: { step: { value: ++frames, content: `step:${frames}` } }, motion: frames < 3 }) };
  const { page, lane, later, mount, step } = rig({ moving });
  mount(1, [{ id: 3, channel: 'panel' }], 'moving');
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', data: {} }]));
  await step(10, { playing: false });
  assert.equal(page.view(1).fields?.step, 1);
  while (later.length) { later.splice(0).forEach(run => run()); await flush(); await flush(); page.frame(); await flush(); }
  assert.equal(page.view(1).fields?.step, 3, 'motion ran to its end, each step a whole install');
});

test('the host’s dated binding and the lane’s intersection meet: a static proof dates the picture alone or beside a followed stream', async () => {
  const proof = [100, 140] as const;
  const alone = rig();
  alone.mount(1, [], 'fields', 0, { dated: boundDated({ hour: 100, follows: true }, proof, false, false, null)! });
  await alone.step(99, { playing: false });
  assert.equal(alone.page.view(1).mode, 'reading', 'nothing before the hour the static read answered');
  await alone.step(120, { playing: false });
  assert.deepEqual([alone.page.view(1).mode, alone.page.view(1).h, alone.page.view(1).through, alone.page.view(1).dim], ['picture', 100, 120, false]);
  await alone.step(150, { playing: false });
  assert.deepEqual([alone.page.view(1).through, alone.page.view(1).dim], [140, true], 'at rest past the proof the picture is held as not current');

  const beside = rig();
  const dated = boundDated({ hour: 100, follows: true }, proof, true, false, null);
  assert.deepEqual(dated, { h: 100, c: 140 }, 'a followed static read is dated by its proof even beside subscriptions');
  beside.mount(1, [{ id: 5, channel: 'followed' }], 'fields', 0, { dated: dated! });
  beside.lane.bindRoute(5, ROUTE, NATIVE);
  beside.lane.receive('follow', message(160, [{ id: 5, hour: 90, answer: rows([1n], [1n]) }], false, NATIVE));
  await beside.step(130, { playing: false });
  assert.deepEqual([beside.page.view(1).mode, beside.page.view(1).h, beside.page.view(1).through], ['picture', 100, 130], 'the join starts at the later of the stream and the static read');
  await beside.step(150, { playing: false });
  assert.deepEqual([beside.page.view(1).through, beside.page.view(1).dim], [140, true], 'and is confirmed only through the static proof, though the stream confirms 160');
  assert.equal(boundDated({ hour: 100 }, null, true, false, null), undefined, 'a pinned read beside subscriptions leaves dating to the stream');
});

test('addendum 3: a widget of two subscriptions installs their join, or waits while either is missing', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }, { id: 7, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE); lane.bindRoute(7, ROUTE, NATIVE);
  lane.receive('follow', message(30, [{ id: 5, hour: 10, answer: { keys: [1n], columns: 1, values: [1n] } }], false, NATIVE));
  await step(25, { playing: false });
  assert.equal(page.view(1).mode, 'reading', 'one of two is not a picture');
  lane.receive('follow', message(30, [{ id: 7, hour: 20, answer: { keys: [1n], columns: 1, values: [2n] } }], false, NATIVE));
  await step(25);
  assert.deepEqual([page.view(1).h, page.view(1).through], [20, 25]);
  mount(2, [{ id: 5, channel: 'followed' }], 'fields', 0, { dated: { h: 28, c: 28 } });
  await step(26);
  assert.equal(page.view(2).mode, 'reading', 'a followed widget whose static read was asked at 28 shows nothing at 26');
  await step(29);
  assert.equal(page.view(2).mode, 'picture', 'and installs once the clock reaches the hour that read answered');
  assert.deepEqual([page.view(2).h, page.view(2).through], [28, 28], 'the picture claims only the hours both its static read and its followed stream prove');
});

test('R3-S2: a finite static proof is never left open to a later confirmation, on the followed or the panel channel', async () => {
  const followed = rig();
  followed.mount(1, [{ id: 5, channel: 'followed' }], 'fields', 0, { dated: { h: 28, c: 28 } });
  followed.lane.bindRoute(5, ROUTE, NATIVE);
  followed.lane.receive('follow', message(28, [{ id: 5, hour: 20, answer: rows([1n], [1n]) }], false, NATIVE));
  await followed.step(28, { playing: false });
  assert.deepEqual([followed.page.view(1).mode, followed.page.view(1).through], ['picture', 28], 'the route confirms exactly the static proof’s end');
  followed.lane.receive('follow', message(30, [], false, NATIVE));
  await followed.step(30, { playing: false });
  assert.equal(followed.page.view(1).through, 28, 'a route confirmation of 30 does not extend a picture whose static read proves only 28');
  assert.equal(followed.page.view(1).dim, true);

  const panel = rig();
  panel.mount(1, [{ id: 3, channel: 'panel', product: 'demo' }], 'fields', 0, { dated: { h: 28, c: 28 } });
  panel.lane.bindRoute(3, ROUTE, NATIVE);
  panel.lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: { shape: 'A' } }, { id: 0, hour: 28, kind: 'completed' }]));
  await panel.step(28, { playing: false });
  assert.deepEqual([panel.page.view(1).mode, panel.page.view(1).through], ['picture', 28]);
  panel.lane.receive('panels', panels([{ id: 0, hour: 30, kind: 'completed' }]));
  await panel.step(30, { playing: false });
  assert.equal(panel.page.view(1).through, 28, 'nor does a panel confirmation');
});

test('R4-S6: an open picture closes when a finite static bound is supplied later, on the followed or the panel channel', async () => {
  const followed = rig();
  followed.mount(1, [{ id: 5, channel: 'followed' }]);
  followed.lane.bindRoute(5, ROUTE, NATIVE);
  followed.lane.receive('follow', message(28, [{ id: 5, hour: 28, answer: rows([1n], [1n]) }], false, NATIVE));
  await followed.step(28, { playing: false });
  assert.deepEqual([followed.page.view(1).mode, followed.page.view(1).through], ['picture', 28], 'an open followed picture confirmed at 28');
  followed.lane.supply(1, { dated: { h: 28, c: 28 } });
  await followed.step(28, { playing: false });
  followed.lane.receive('follow', message(30, [], false, NATIVE));
  await followed.step(30, { playing: false });
  assert.equal(followed.page.view(1).through, 28, 'a route confirmation of 30 does not extend it once the static read proves only 28');
  assert.equal(followed.page.view(1).dim, true);

  const panel = rig();
  panel.mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  panel.lane.bindRoute(3, ROUTE, NATIVE);
  panel.lane.receive('panels', panels([{ id: 3, hour: 28, kind: 'data', stepped: true, data: { shape: 'A' } }, { id: 0, hour: 28, kind: 'completed' }]));
  await panel.step(28, { playing: false });
  assert.deepEqual([panel.page.view(1).mode, panel.page.view(1).through], ['picture', 28]);
  panel.lane.supply(1, { dated: { h: 28, c: 28 } });
  await panel.step(28, { playing: false });
  panel.lane.receive('panels', panels([{ id: 0, hour: 30, kind: 'completed' }]));
  await panel.step(30, { playing: false });
  assert.equal(panel.page.view(1).through, 28, 'nor does a panel confirmation');
});

test('R5-S4: paused on an open picture first confirmed at 30, a finite proof [28, 28] invalidates it, on the followed or the panel channel', async () => {
  const followed = rig();
  followed.mount(1, [{ id: 5, channel: 'followed' }]);
  followed.lane.bindRoute(5, ROUTE, NATIVE);
  followed.lane.receive('follow', message(30, [{ id: 5, hour: 30, answer: rows([1n], [1n]) }], false, NATIVE));
  await followed.step(30, { playing: false });
  assert.deepEqual([followed.page.view(1).mode, followed.page.view(1).h, followed.page.view(1).through], ['picture', 30, 30], 'an open followed picture first and confirmed at 30');
  followed.lane.supply(1, { dated: { h: 28, c: 28 } });
  await followed.step(30, { playing: false });
  assert.notEqual(followed.page.view(1).mode, 'picture', 'its hours and its proof do not meet: the page shows no current picture at 30');

  const panel = rig();
  panel.mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  panel.lane.bindRoute(3, ROUTE, NATIVE);
  panel.lane.receive('panels', panels([{ id: 3, hour: 30, kind: 'data', stepped: true, data: { shape: 'A' } }, { id: 0, hour: 30, kind: 'completed' }]));
  await panel.step(30, { playing: false });
  assert.deepEqual([panel.page.view(1).mode, panel.page.view(1).h], ['picture', 30]);
  panel.lane.supply(1, { dated: { h: 28, c: 28 } });
  await panel.step(30, { playing: false });
  assert.notEqual(panel.page.view(1).mode, 'picture', 'nor on the panel channel');
});

const at: LaneView = {
  next: (_scene, hour) => (Math.floor(hour / 24) + 1) * 24,
  paint: (input): Painted => ({ surfaces: [], fields: { at: { value: input.hour, content: `at:${input.hour}` } }, motion: false }),
};

test('astra 2: each widget selects, paints and dates at its own target: an offset of +100, and a pinned table at its pin', async () => {
  const { page, lane, mount, step } = rig({ at });
  mount(1, [{ id: 3, channel: 'panel' }], 'at', 0, { offset: 100 });
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 100, kind: 'data', stepped: true, data: {} }, { id: 0, hour: 110, kind: 'completed' }]));
  mount(2, [], 'at', 0, { pin: 50, dated: { h: 50, c: 50 } });
  await step(5, { playing: false });
  assert.deepEqual([page.view(1).mode, page.view(1).h, page.view(1).through, page.view(1).fields?.at], ['picture', 100, 105, 100], 'the offset widget at page hour 5 shows hour 105');
  assert.deepEqual([page.view(2).mode, page.view(2).h, page.view(2).fields?.at], ['picture', 50, 50], 'the pinned table paints at its pin, never at -Infinity');
});

test('astra 4: an empty same-seek reset withdraws the follow picture though no widget repaints', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [10n]) }], false, NATIVE));
  await step(10, { playing: false });
  assert.equal(page.view(1).h, 10);
  lane.receive('follow', message(10, [], true, NATIVE));
  await step(10);
  assert.equal(page.view(1).mode, 'reading', 'the reset withdrew what it invalidated; nothing replaced it');
});

test('astra 4: a lane bound after an earlier reset dates its entries with the reset every lane saw', async () => {
  const lanes: Lane<Fake>[] = [];
  const page = new Page<Fake>({ lanes: 2, post: (to, m) => lanes.forEach((lane, index) => {
    if (to !== 'all' && to !== index) return;
    if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
  }) });
  for (let index = 0; index < 2; index++) lanes.push(new Lane<Fake>({ lane: index, lanes: 2, views: () => fieldsView(() => undefined), crop: async () => [],
    present: presentation => page.offer(presentation), release: () => {}, fail: () => {}, later: () => {} }));
  const send = (bytes: Uint8Array) => { const split = splitForLanes(bytes, 2); if (split.kind !== 'failure') split.lanes.forEach((part, index) => { if (part) lanes[index]!.receive('follow', part); }); };
  const step = async (T: number) => { page.publish(T, { playing: false }); await flush(); await flush(); page.frame(); await flush(); };
  for (const [widget, sub, index] of [[1, 4, 0], [2, 5, 1]] as const) {
    lanes[index]!.follow(sub, 'followed');
    lanes[index]!.mount(widget, [sub], 'fields');
    lanes[index]!.bind(widget, { generation: 1, surfaces: [], offset: 0, pin: null });
    page.mount(widget, { route: ROUTE, lane: index, surfaces: [] });
  }
  lanes[0]!.bindRoute(4, ROUTE, NATIVE);
  send(message(10, [{ id: 4, answer: rows([1n], [1n]) }], false, NATIVE));
  send(message(11, [{ id: 4, answer: rows([1n], [2n]) }], true, NATIVE));
  await step(11);
  assert.equal(page.view(1).h, 11);
  lanes[1]!.bindRoute(5, ROUTE, NATIVE);
  send(message(12, [{ id: 5, answer: rows([1n], [3n]) }], false, NATIVE));
  await step(12);
  assert.deepEqual([page.view(2).mode, page.view(2).h, page.stats.rejected.reset], ['picture', 12, undefined], 'the late lane counted the reset it saw before it was bound');
});

test('astra 5: after a rewind refuses kept content, the lane resends the whole picture and recovers', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 50, kind: 'data', stepped: true, data: { shape: 'A' } }, { id: 0, hour: 60, kind: 'completed' }]));
  await step(55, { playing: false });
  assert.equal(page.view(1).h, 50);
  await step(5, { playing: false, seek: true });
  assert.equal(page.view(1).mode, 'reading');
  lane.receive('panels', panels([{ id: 3, hour: 3, kind: 'data', stepped: true, data: { shape: 'A' } }, { id: 0, hour: 5, kind: 'completed' }], 1));
  for (let i = 0; i < 4; i++) await step(5);
  assert.deepEqual([page.view(1).mode, page.view(1).h], ['picture', 3]);
});

test('astra 6: an uncovered paused step keeps the installed content and never paints a day past the confirmation', async () => {
  const hours: number[] = [];
  const day: LaneView = { next: at.next, paint: (input): Painted => { hours.push(input.hour); return { surfaces: [], fields: { day: { value: Math.floor(input.hour / 24), content: `day:${input.hour}` } }, motion: false }; } };
  const { page, lane, mount, step } = rig({ day });
  mount(1, [{ id: 3, channel: 'panel' }], 'day');
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 96, kind: 'data', stepped: true, data: {} }, { id: 0, hour: 140, kind: 'completed' }]));
  await step(124, { playing: false });
  await step(150);
  assert.deepEqual([[...new Set(hours)], page.view(1).fields?.day, page.view(1).dim], [[120], 5, true], 'confirmed only through 140, the day of 144 is not painted');
  lane.receive('panels', panels([{ id: 0, hour: 150, kind: 'completed' }]));
  await step(150);
  assert.deepEqual([[...new Set(hours)], page.view(1).fields?.day, page.view(1).dim], [[120, 144], 6, false]);
});

test('astra 9: a buffered answer replays once with its own confirmation, never extending another reading', () => {
  const { lane, mount, failures } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.follow(7, 'followed');
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.receive('follow', message(30, [{ id: 5, hour: 10, answer: rows([1n], [1n]) }], false, NATIVE));
  lane.receive('follow', withLimits(message(40, [{ id: 7, hour: 35, answer: rows([1n], [2n]) }], false, NATIVE), [[5, 30]]));
  assert.equal(lane.facts.select(5, 40)?.c, 30);
  lane.bindRoute(7, ROUTE, NATIVE);
  assert.deepEqual([lane.facts.select(5, 40)?.c, lane.facts.select(7, 40)?.h, lane.facts.select(7, 40)?.c], [30, 35, 40], 'replaying 7 leaves the limit of 5 alone');
  assert.deepEqual(failures, []);
});

test('astra 9: every buffered change of an unbound reading keeps its base; nothing is silently dropped', async () => {
  const { page, lane, mount, step, failures } = rig();
  mount(1, [{ id: 7, channel: 'followed' }]);
  lane.receive('follow', message(10, [{ id: 7, answer: rows([1n, 2n], [10n, 20n]) }], false, NATIVE));
  for (let hour = 11; hour <= 20; hour++) lane.receive('follow', message(hour, [{ id: 7, answer: rows([2n], [BigInt(hour)], { changed: true }) }], false, NATIVE));
  lane.bindRoute(7, ROUTE, NATIVE);
  await step(20, { playing: false });
  assert.deepEqual([failures, page.view(1).h, valueOf(page.view(1), 7, 0), valueOf(page.view(1), 7, 1)], [[], 20, 10n, 20n]);
});

test('astra 10: a widget named failed stays failed through queued work, and its lane stops offering it until a new binding', async () => {
  const { page, lane, mount, step, presented } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [1n]) }], false, NATIVE));
  await step(10, { playing: false });
  page.name(1, 'failed', 'The registration was refused');
  lane.receive('follow', message(11, [{ id: 5, answer: rows([1n], [2n]) }], false, NATIVE));
  for (let hour = 11; hour < 15; hour++) await step(hour);
  assert.deepEqual([page.view(1).mode, page.view(1).reason], ['failed', 'The registration was refused']);
  assert.ok(presented.flatMap(p => p.entries).filter(e => e.h >= 11).length <= 1, 'one refused offer, not one a frame');
  page.rebind(1, { subscription: true });
  lane.bind(1, { generation: 2, surfaces: [], offset: 0, pin: null });
  await step(15);
  assert.deepEqual([page.view(1).mode, page.view(1).h], ['picture', 11]);
});

test('astra 14: when the budget serves one widget a frame, stable widgets take turns', async () => {
  const { page, lane, mount, step, presented } = rig({}, { budget: 0 });
  for (const widget of [1, 2, 3]) mount(widget, [{ id: widget * 2 + 1, channel: 'panel' }]);
  for (const widget of [1, 2, 3]) lane.bindRoute(widget * 2 + 1, ROUTE, NATIVE);
  lane.receive('panels', panels([1, 2, 3].map(widget => ({ id: widget * 2 + 1, hour: 10, kind: 'data' as const, stepped: true, data: {} }))));
  for (let hour = 10; hour < 19; hour++) {
    lane.receive('panels', panels([
      ...[1, 2, 3].map(widget => ({ id: widget * 2 + 1, hour, kind: 'data' as const, stepped: true, data: { shape: `h${hour}` } })),
      { id: 0, hour, kind: 'completed' },
    ]));
    await step(hour, { playing: false });
  }
  const served = new Map<number, number>();
  for (const entry of presented.flatMap(p => p.entries)) served.set(entry.widget, (served.get(entry.widget) ?? 0) + 1);
  assert.ok([1, 2, 3].every(widget => (served.get(widget) ?? 0) >= 2), `served ${JSON.stringify([...served])}`);
  assert.equal(page.view(3).mode, 'picture');
});

test('should-fix a: a pointer hit-tests the installed picture, never a candidate the page has not accepted', async () => {
  const view: LaneView = { paint: (input): Painted => ({ surfaces: [], fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } }, motion: false,
    hit: () => `row:${input.pictures[0]!.h}` }) };
  const { page, lane, mount, step, hits, presented } = rig({ view });
  mount(1, [{ id: 3, channel: 'panel' }], 'view');
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }]));
  await step(10, { playing: false });
  lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: {} }]));
  page.publish(20, { playing: false });
  await flush(); await flush();
  lane.pointer(1, 0, [1, 1], page.view(1).id);
  await flush(); await flush();
  assert.deepEqual(hits, ['row:10']);
  const offered = presented.at(-1)!.entries.at(-1)!;
  lane.pointer(1, 0, [1, 1], offered.id);
  assert.deepEqual(hits, ['row:10', 'row:20'], 'a pointer hit-tests the picture it names (the page names the one it installed), even before the lane hears its acknowledgement');
});

test('R3-S6: a same-owner rebind during a deferred acknowledgement keeps the picture the page shows as the hit target', async () => {
  const view: LaneView = { paint: (input): Painted => ({ surfaces: [], fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } }, motion: false,
    hit: () => `row:${input.pictures[0]!.h}` }) };
  const { page, lane, mount, step, hits, presented } = rig({ view });
  mount(1, [{ id: 3, channel: 'panel' }], 'view');
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }]));
  await step(10, { playing: false });
  lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: {} }]));
  page.publish(20, { playing: false });
  await flush(); await flush();
  page.frame();
  const shown = presented.at(-1)!.entries.at(-1)!;
  assert.equal(page.view(1).id, shown.id, 'the page installed B; its acknowledgement waits for the next frame');
  lane.bind(1, { generation: 2, surfaces: [], offset: 0, pin: null });
  lane.pointer(1, 0, [1, 1], shown.id);
  assert.deepEqual(hits, ['row:20'], 'the lane hit-tests B, which the page still shows, not the A it last acknowledged');
  await step(20);
  lane.pointer(1, 0, [1, 1], shown.id);
  assert.deepEqual(hits, ['row:20', 'row:20'], 'and still after B’s acknowledgement arrives under the old binding');
});

test('changed rows still in flight from before a rewind never merge onto the new stream, and an early stream waits for its seek', async () => {
  const { page, lane, mount, step, failures } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n, 2n], [10n, 20n]) }], false, NATIVE, 0));
  await step(10, { playing: false });
  await step(4, { playing: false, seek: true });
  lane.receive('follow', message(4, [{ id: 5, answer: { notRecorded: 'Not recorded before 5' } }], true, NATIVE, 1));
  lane.receive('follow', message(11, [{ id: 5, answer: rows([2n], [21n], { changed: true }) }], false, NATIVE, 0));
  lane.receive('follow', message(8, [{ id: 5, answer: rows([1n, 2n], [80n, 90n]) }], true, NATIVE, 2));
  lane.receive('follow', message(9, [{ id: 5, answer: rows([2n], [91n], { changed: true }) }], false, NATIVE, 2));
  await step(4);
  assert.deepEqual([failures, page.view(1).mode], [[], 'refused'], 'the old stream\'s change is dropped, not merged onto a not-recorded answer, which stays a named refusal');
  await step(9, { playing: false, seek: true });
  assert.deepEqual([page.view(1).h, valueOf(page.view(1), 5, 0), valueOf(page.view(1), 5, 1)], [9, 80n, 91n], 'the early stream merges onto its own table once its seek is current');
});

test('review 4 finding 4: a buffered message replays as native sent it: a limit before its answer still limits it, and a same-batch completion still covers a stepped panel', () => {
  const { lane, mount } = rig();
  mount(1, [{ id: 7, channel: 'followed' }]);
  const answer = message(40, [{ id: 7, hour: 10, answer: rows([1n], [1n]) }], false, NATIVE);
  const limitFirst = new Uint8Array(answer.length + 16), view = new DataView(limitFirst.buffer);
  limitFirst.set(answer.subarray(0, 24)); limitFirst.set(answer.subarray(24), 40);
  view.setUint32(12, 2, true); view.setUint32(24, 7, true); view.setInt32(28, 10, true);
  lane.receive('follow', limitFirst);
  lane.bindRoute(7, ROUTE, NATIVE);
  assert.deepEqual([lane.facts.select(7, 40)?.h, lane.facts.select(7, 40)?.c], [10, 10], 'confirmed through its limit of 10, never through the envelope hour 40');
  mount(2, [{ id: 3, channel: 'panel' }]);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }, { id: 0, hour: 40, kind: 'completed' }]));
  lane.bindRoute(3, ROUTE, NATIVE);
  assert.deepEqual([lane.facts.select(3, 40)?.h, lane.facts.select(3, 40)?.c], [10, 40], 'the completion in its own batch covers the stepped panel');
});

test('review 4 finding 8: a seek that carries the shown picture keeps its bytes charged until a new picture replaces it', async () => {
  const { page, lane, mount, step, released } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE);
  const first = panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: { shape: 'A', pad: 'x'.repeat(4000) } }, { id: 0, hour: 20, kind: 'completed' }]);
  lane.receive('panels', first);
  await step(12, { playing: false });
  assert.equal(page.view(1).h, 10);
  const total = () => released.reduce((sum, bytes) => sum + bytes, 0);
  await step(14, { playing: false, seek: true });
  assert.equal(page.view(1).h, 10, 'the carried picture still shows');
  assert.ok(total() < 4000, `released ${total()} while the carried picture still holds its fact`);
  lane.receive('panels', panels([{ id: 3, hour: 14, kind: 'data', stepped: true, data: { shape: 'A', n: 2 } }], 1));
  await step(14); await step(14);
  assert.equal(page.view(1).h, 14);
  assert.ok(total() >= 4000, 'once replaced, its bytes come back');
});

test('should-fix: an acknowledgement for an entry sent before the one in flight still makes it the installed picture that pointers hit-test', async () => {
  const view: LaneView = { paint: (input): Painted => ({ surfaces: [], fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } }, motion: false,
    hit: () => `row:${input.pictures[0]!.h}` }) };
  const { page, lane, mount, step, hits } = rig({ view });
  mount(1, [{ id: 3, channel: 'panel' }], 'view');
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }]));
  await step(10, { playing: false });
  lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: {} }]));
  page.publish(20, { playing: false });
  await flush(); await flush();
  page.frame();
  lane.receive('panels', panels([{ id: 3, hour: 30, kind: 'data', stepped: true, data: {} }]));
  page.publish(30, { playing: false });
  await flush(); await flush();
  assert.equal(page.view(1).h, 20, 'the page shows 20 while 30 is in flight');
  lane.pointer(1, 0, [1, 1]);
  assert.deepEqual(hits, ['row:20']);
});

test('review 5 finding 3: the facts of an entry sent but not yet acknowledged stay charged until the page answers for it', async () => {
  const { lane, mount, released, presented } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  const frame = (serial: number, target: number, refused: FrameRequest['refused'] = []): FrameRequest =>
    ({ type: 'frame', serial, now: target, target, exact: true, seek: 0, speed: 0, installed: [], refused, inspect: [] });
  const big = rows(Array.from({ length: 200 }, (_, at) => BigInt(at + 1)), Array.from({ length: 200 }, () => 1n));
  lane.receive('follow', message(10, [{ id: 5, answer: big }], false, NATIVE));
  lane.frame(frame(1, 10)); await flush(); await flush();
  lane.receive('follow', message(20, [{ id: 5, answer: rows([1n], [2n]) }], false, NATIVE));
  lane.receive('follow', message(25, [{ id: 5, answer: rows([1n], [3n]) }], false, NATIVE));
  lane.frame(frame(2, 30)); await flush(); await flush();
  const total = () => released.reduce((sum, bytes) => sum + bytes, 0);
  assert.equal(presented.flatMap(p => p.entries).length, 2);
  assert.ok(total() < 200 * 16, `released ${total()} while the page may still install the first entry`);
  lane.frame(frame(3, 30, [{ widget: 1, id: presented[0]!.entries[0]!.id, reason: 'superseded' }])); await flush(); await flush();
  assert.ok(total() >= 200 * 16, 'once the page refuses it, its facts come back');
});

test('review 5 finding 3: an entry whose crop failed is retired and reported via failWidget without emitting a failure entry', async () => {
  const view: LaneView = { paint: (): Painted => ({ surfaces: [{ source: { width: 1, height: 1, content: 'p' } as never, content: 'p' }], fields: {}, motion: false }) };
  const { page, lane, mount, step, failures, widgetFailures, presented } = rig({ view }, { crop: async () => { throw new Error('canvas lost'); } });
  mount(1, [{ id: 3, channel: 'panel' }], 'view', 1);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }, { id: 0, hour: 20, kind: 'completed' }]));
  await step(12, { playing: false });
  assert.deepEqual(failures, [], 'the lane keeps serving');
  assert.deepEqual(widgetFailures, [[1, 'drawing failed: canvas lost']]);
  assert.deepEqual([page.view(1).mode, page.view(1).reason], ['failed', 'drawing failed: canvas lost']);
  assert.deepEqual(presented.flatMap(p => p.entries), [], 'no failure picture entry is emitted');
});

test('review 5 finding 5: a limit buffered before its route was bound gives its header back when it lands', () => {
  const { lane, mount, released } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.receive('follow', withLimits(message(40, [], false, NATIVE), [[5, 30]]));
  assert.equal(released.reduce((sum, bytes) => sum + bytes, 0), 0, 'held until its route is known');
  lane.bindRoute(5, ROUTE, NATIVE);
  assert.equal(released.reduce((sum, bytes) => sum + bytes, 0), 16);
});

test('review 6 A: a changed row naming a key its base lacks gives its whole charge back when the merge refuses it', async () => {
  const { page, lane, mount, step, released, failures } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  const total = () => released.reduce((sum, bytes) => sum + bytes, 0);
  const base = message(10, [{ id: 5, answer: rows([1n, 2n], [1n, 2n]) }], false, NATIVE);
  lane.receive('follow', base);
  const before = total();
  const changed = message(20, [{ id: 5, answer: rows([9n], [3n], { changed: true }) }], false, NATIVE);
  lane.receive('follow', changed);
  const chargeOf = (bytes: Uint8Array) => {
    const rec = bytes.subarray(24), dv = new DataView(rec.buffer, rec.byteOffset);
    return 16 + 2 * Math.max(dv.getUint32(8, true), dv.getUint32(12, true));
  };
  assert.equal(total() - before, chargeOf(changed) + chargeOf(base), 'the refused record and dropped unshown base are both released');
  await step(20, { playing: false });
  assert.deepEqual(failures, [], 'the lane stays up while the faulted subscription fails its widget');
  assert.equal(page.view(1).mode, 'failed');
  assert.match(page.view(1).reason ?? '', /does not hold/);
});

test('review 7 C: a whole-answer refusal installs as a named refusal; one refused component of a join stays inside its picture', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  mount(2, [{ id: 5, channel: 'followed' }, { id: 7, channel: 'followed' }]);
  for (const sub of [5, 7]) lane.bindRoute(sub, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 5, answer: { notRecorded: 'The ranking was not recorded' } }, { id: 7, answer: rows([1n], [2n]) }], false, NATIVE));
  await step(10, { playing: false });
  assert.deepEqual([page.view(1).mode, page.view(1).reason], ['refused', 'The ranking was not recorded']);
  assert.equal(page.view(2).mode, 'picture', 'the joined widget shows its other column with the refused one inside');
  const fake = (widget: number) => ({ view: () => page.view(widget), status: () => ({ status: 'followed' as const }), onInstall: () => () => {} }) as unknown as Pipeline;
  assert.deepEqual([componentReading(page.view(2), 5), componentReading(page.view(2), 7).state], [{ state: 'refused', reason: 'The ranking was not recorded' }, 'covered'],
    'review 8 2: the refused component has its own outcome inside the joined picture');
  const refused = installedEntry(fake(2), 2, 5), whole = installedEntry(fake(1), 1, 5);
  assert.deepEqual([refused.status, refused.reason, whole.status, whole.reason, installedEntry(fake(2), 2, 7).status],
    ['unavailable', 'The ranking was not recorded', 'unavailable', 'The ranking was not recorded', 'followed'], 'review 8 2: both refusals reach the shared entry its consumers read');
});

test('review 7 E: a followed message that fails to decode releases its charge at once, even at an idle paused lane', () => {
  const { lane, mount, released } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  const valid = message(10, [{ id: 5, answer: rows([1n], [2n]) }], false, NATIVE);
  const bad = new Uint8Array(valid.length + 1);
  bad.set(valid);
  assert.throws(() => lane.receive('follow', bad));
  assert.ok(released.reduce((sum, bytes) => sum + bytes, 0) > 0, 'the refused record was handed back without waiting for another frame');
});

test('review 8 1: a reset that arrives before its route is bound waits for its seek, so an older one never drops the held records', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  await step(10, { playing: false, seek: true });
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [10n]) }], false, NATIVE, 1));
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [1n]) }], true, NATIVE, 0));
  lane.bindRoute(5, ROUTE, NATIVE, 1);
  await step(10);
  assert.deepEqual([page.view(1).mode, valueOf(page.view(1), 5)], ['picture', 10n]);
});

test('review 8 2: a removed component keeps its facts charged while the page still shows them, and they come back once the new picture installs', async () => {
  const { page, lane, released, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }, { id: 6, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.bindRoute(6, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [10n]) }, { id: 6, answer: rows([1n], [20n]) }], false, NATIVE));
  await step(10, { playing: false });
  await step(10);
  assert.equal(page.view(1).mode, 'picture');
  const before = released.reduce((sum, bytes) => sum + bytes, 0);
  lane.mount(1, [5], 'fields');
  lane.unfollow(6);
  assert.equal(released.reduce((sum, bytes) => sum + bytes, 0), before, 'nothing shown is refunded');
  page.rebind(1, { subscription: true });
  lane.bind(1, { generation: page.widgets.get(1)!.binding, surfaces: [], offset: 0, pin: null });
  await step(10);
  await step(10);
  assert.ok(released.reduce((sum, bytes) => sum + bytes, 0) > before, 'the removed facts come back once the joined picture replaces them');
});

test('review 9: a lane without the route\'s follower keeps no packets after a reset, and gives each charge back', () => {
  const { lane, released } = rig();
  lane.follow(8, 'followed');
  for (let hour = 0; hour < 50; hour++) lane.receive('follow', message(hour, [{ id: 5, answer: rows([1n], [BigInt(hour)]) }], hour % 10 === 0, NATIVE, 1));
  assert.deepEqual([(lane as unknown as { early: unknown[] }).early.length, released.length > 0], [0, true], 'the other lane\'s records are released, never queued without limit');
});

test('review 11: a lane that first binds on a later seek takes the stamped reset count, so the page withdraws pre-reset pictures route-wide', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  await step(10, { playing: false, seek: true });
  lane.receive('follow', message(10, [], true, NATIVE, 0), 1);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [10n]) }], false, NATIVE, 1), 1);
  lane.bindRoute(5, ROUTE, NATIVE, 1);
  await step(10);
  assert.deepEqual([page.view(1).mode, page.widgets.get(1)!.installed?.gen.reset], ['picture', 1], 'the seek-0 reset counted on another lane is in this lane\'s count too');
});

test('review 12: a carried answer keeps the reset generation it was applied under, so a stamp from a discarded packet never reinstalls it', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  mount(2, [{ id: 6, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE, 0);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [10n]) }], false, NATIVE, 0), 0);
  await step(10, { playing: false });
  await step(10, { playing: false, seek: true });
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [99n]) }], true, NATIVE, 0), 1);
  page.offer({ lane: 0, serial: 900, resets: [[ROUTE, 1]], entries: [] });
  lane.bindRoute(6, ROUTE, NATIVE, 1);
  page.rebind(1, {});
  lane.bind(1, { generation: page.widgets.get(1)!.binding, surfaces: [], offset: 0, pin: null });
  await step(10);
  await step(10);
  const view = page.view(1);
  assert.ok(view.mode !== 'picture' || page.widgets.get(1)!.installed!.gen.reset >= 1 && valueOf(view, 5) !== 10n,
    'the answer from before the reset another lane reported is not shown again: ' + JSON.stringify([view.mode, page.widgets.get(1)?.installed?.gen, view.mode === 'picture' ? String(valueOf(view, 5)) : null]));
});

test('a seek that reuses a panel cell group receives its reset table before the panel and merges later changes', async () => {
  const { page, lane, mount, step, failures } = rig();
  const CELL_A = 0x8000_0001;
  const panelData = (n: number) => ({
    shape: 'S', n,
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: CELL_A }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  });
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: panelData(1) }, { id: 0, hour: 10, kind: 'completed' }], 0));
  lane.receive('follow', message(10, [{ id: CELL_A, hour: 10, answer: rows([1n], [100n]) }], true, NATIVE, 0));
  await step(10, { playing: false });
  assert.equal(page.view(1).mode, 'picture');
  await step(50, { playing: false, seek: true });
  await step(50, { playing: false });
  lane.receive('follow', message(50, [{ id: CELL_A, hour: 50, answer: rows([1n], [200n]) }], true, NATIVE, 1));
  lane.receive('panels', panels([{ id: 3, hour: 50, kind: 'data', stepped: true, data: panelData(2) }, { id: 0, hour: 50, kind: 'completed' }], 1));
  lane.receive('follow', message(51, [{ id: CELL_A, hour: 51, answer: rows([1n], [205n], { changed: true }) }], false, NATIVE, 1));
  lane.receive('panels', panels([{ id: 0, hour: 51, kind: 'completed' }], 1));
  await step(51, { playing: false });
  await step(51, { playing: false });
  assert.deepEqual(failures, []);
  assert.equal(page.view(1).mode, 'picture');
  const live = page.view(1).fields?.['live:3'] as Map<string, unknown> | undefined;
  assert.equal(live?.get('r0\u0000k'), 205);
});

test('a protocol fault on one subscription fails only widgets using it, sticks across seeks, and leaves sibling widgets serving', async () => {
  const { page, lane, mount, step, failures, subFailures, released } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  mount(2, [{ id: 7, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.bindRoute(7, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n, 2n], [10n, 20n]) }, { id: 7, answer: rows([1n], [70n]) }], false, NATIVE));
  await step(10, { playing: false });
  assert.deepEqual([page.view(1).mode, page.view(2).mode], ['picture', 'picture']);
  lane.receive('follow', message(20, [{ id: 5, answer: rows([9n], [99n], { changed: true }) }, { id: 7, answer: rows([1n], [75n], { changed: true }) }], false, NATIVE));
  await step(20, { playing: false });
  assert.deepEqual(failures, [], 'the lane keeps serving when one subscription faults');
  assert.equal(subFailures.length, 1);
  assert.equal(subFailures[0]![0], 5);
  assert.equal(page.view(1).mode, 'failed');
  assert.match(page.view(1).reason ?? '', /does not hold/);
  assert.deepEqual([page.view(2).mode, valueOf(page.view(2), 7)], ['picture', 75n]);
  const before = released.reduce((sum, b) => sum + b, 0);
  lane.receive('follow', message(30, [{ id: 5, answer: rows([1n, 2n], [30n, 40n]) }], true, NATIVE));
  assert.ok(released.reduce((sum, b) => sum + b, 0) > before, 'later records for a failed subscription return credit immediately');
  assert.equal(subFailures.length, 1, 'failSub is posted only once per subscription');
  await step(30, { playing: false });
  assert.equal(page.view(1).mode, 'failed', 'a decode or merge fault stays failed on the same seek');
  await step(30, { playing: false, seek: 1 });
  lane.receive('follow', message(30, [{ id: 5, answer: rows([1n, 2n], [30n, 40n]) }, { id: 7, answer: rows([1n], [80n]) }], true, NATIVE, 1));
  await step(30, { playing: false });
  assert.equal(page.view(1).mode, 'failed', 'a subscription fault is sticky across seeks until unfollow/resubscription');
  assert.deepEqual([page.view(2).mode, valueOf(page.view(2), 7)], ['picture', 80n]);
});

test('quiet frames visit zero unchanged widgets in serve and prune across 114 widgets, and K changed visits O(K)', async () => {
  const { page, lane, mount, step } = rig({}, { clock: () => 0 });
  const initial: { id: number; answer: ReturnType<typeof rows> }[] = [];
  const initialPanels: PanelRow[] = [];
  for (let i = 1; i <= 90; i++) {
    const sub = i * 2 + 1;
    mount(i, [{ id: sub, channel: 'followed' }]);
    lane.bindRoute(sub, ROUTE, NATIVE);
    initial.push({ id: sub, answer: rows([1n], [BigInt(i)]) });
  }
  for (let i = 91; i <= 106; i++) {
    const sub = i * 2 + 1;
    mount(i, [{ id: sub, channel: 'panel', product: 'demo' }]);
    lane.bindRoute(sub, ROUTE, NATIVE);
    initialPanels.push({ id: sub, hour: 10, kind: 'data', stepped: true, data: { shape: `P${i}` } });
  }
  for (let i = 107; i <= 114; i++) {
    const sub = i * 2 + 1;
    mount(i, [{ id: sub, channel: 'panel', product: 'demo' }]);
    lane.bindRoute(sub, ROUTE, NATIVE);
    initialPanels.push({ id: sub, hour: 10, kind: 'data', stepped: false, data: { shape: `NP${i}` } });
  }
  initialPanels.push({ id: 0, hour: 10, kind: 'completed' });
  lane.receive('follow', message(10, initial, false, NATIVE));
  lane.receive('panels', panels(initialPanels, 0));
  await step(10, { playing: true, step: 24, speed: 1440 });
  lane.receive('follow', message(34, initial.map(item => ({ id: item.id, answer: rows([1n], [BigInt(item.id + 1000)], { changed: true }) })), false, NATIVE));
  lane.receive('panels', panels([{ id: 0, hour: 34, kind: 'completed' }], 0));
  await step(34, { playing: true, step: 24, speed: 1440 });
  await step(58, { playing: true, step: 24, speed: 1440 });
  const beforeVisited = lane.stats.visited;
  const beforePruned = lane.stats.pruned;
  const beforeCompletionVisits = (lane.facts as unknown as { stats?: { completionVisits: number } }).stats?.completionVisits ?? -1;
  for (let h = 82; h <= 202; h += 24) {
    lane.receive('follow', message(h, [], false, NATIVE));
    lane.receive('panels', panels([{ id: 0, hour: h, kind: 'completed' }], 0));
    await step(h, { playing: true, step: 24, speed: 1440 });
  }
  assert.equal(lane.stats.visited - beforeVisited, 0, `quiet frames visited ${lane.stats.visited - beforeVisited} widgets`);
  assert.equal(lane.stats.pruned - beforePruned, 0, `quiet frames pruned ${lane.stats.pruned - beforePruned} subs`);
  assert.equal(((lane.facts as unknown as { stats?: { completionVisits: number } }).stats?.completionVisits ?? -1) - beforeCompletionVisits, 0, 'completion-only packets must do O(1) fact-store work with zero subscription visits');
  const beforeKVisited = lane.stats.visited;
  lane.receive('follow', message(224, [
    { id: 3, answer: rows([1n], [901n], { changed: true }) },
    { id: 5, answer: rows([1n], [902n], { changed: true }) },
  ], false, NATIVE));
  await step(224, { playing: true, step: 24, speed: 1440 });
  assert.ok(lane.stats.visited - beforeKVisited <= 4, `visited ${lane.stats.visited - beforeKVisited} widgets for 2 changed subs`);
  assert.deepEqual([valueOf(page.view(1), 3), valueOf(page.view(2), 5)], [901n, 902n]);
});

test('one expensive widget on a lane does not starve cheap widgets across frames', async () => {
  let clock = 0;
  const views: Record<string, LaneView> = {
    heavy: { paint: (input): Painted => { clock += 20; return { surfaces: [], fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } }, motion: false }; } },
    cheap: { paint: (input): Painted => { clock += 0.2; return { surfaces: [], fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } }, motion: false }; } },
  };
  const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => { if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target); } });
  const lane = new Lane<Fake>({
    lane: 0, lanes: 1, views: kind => views[kind]!, crop: async () => [],
    present: presentation => page.offer(presentation), release: () => {}, fail: () => {}, later: () => {},
    budget: 10, clock: () => clock,
  });
  lane.follow(3, 'followed'); lane.bindRoute(3, ROUTE, NATIVE);
  lane.mount(1, [3], 'heavy'); lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
  page.mount(1, { route: ROUTE, lane: 0, surfaces: [] });
  for (let id = 2; id <= 5; id++) {
    const sub = id * 2 + 1;
    lane.follow(sub, 'followed'); lane.bindRoute(sub, ROUTE, NATIVE);
    lane.mount(id, [sub], 'cheap'); lane.bind(id, { generation: 1, surfaces: [], offset: 0, pin: null });
    page.mount(id, { route: ROUTE, lane: 0, surfaces: [] });
  }
  for (let hour = 10; hour <= 60; hour += 10) {
    lane.receive('follow', message(hour, [3, 5, 7, 9, 11].map(id => ({ id, answer: rows([1n], [BigInt(hour)]) })), false, NATIVE));
    page.publish(hour, { playing: false }); await flush(); await flush(); page.frame(); await flush();
  }
  assert.ok((page.view(1).h ?? 0) >= 20, `heavy widget 1 starved at ${page.view(1).h}`);
  for (let id = 2; id <= 5; id++) assert.ok((page.view(id).h ?? 0) >= 50, `cheap widget ${id} lagged at ${page.view(id).h}`);
  lane.receive('follow', message(70, [3, 5, 7, 9, 11].map(id => ({ id, answer: rows([1n], [70n]) })), false, NATIVE));
  for (let step = 0; step < 3; step++) {
    page.publish(70, { playing: false }); await flush(); await flush(); page.frame(); await flush();
  }
  for (let id = 1; id <= 5; id++) assert.equal(page.view(id).h, 70, `widget ${id} caught up after budget break`);
});

test('S3: a changed record arriving while crop is suspended never certifies the old pixels at the newer hour', async () => {
  let releaseCrop!: () => void;
  const views: Record<string, LaneView> = {
    canvas: {
      paint: (input): Painted => {
        const answer = input.pictures[0]!.parts[0]!.value;
        const val = answer.kind === 'answer' && answer.answer.kind !== 'notRecorded' ? cellValue(answer.answer, 0) : -1n;
        return { surfaces: [{ source: { width: 1, height: 1, content: `v:${val}` } as unknown as { width: number; height: number }, content: `v:${val}` }], fields: {}, motion: false };
      },
    },
  };
  const { page, lane, mount } = rig(views, {
    crop: sources => new Promise(resolve => {
      releaseCrop = () => resolve(sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: 1, close() { this.closed = true; } })));
    }),
  });
  mount(1, [{ id: 3, channel: 'followed' }], 'canvas', 1);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 3, answer: rows([1n], [100n]) }], false, NATIVE));
  page.publish(10, { playing: false });
  await flush();
  lane.receive('follow', message(20, [{ id: 3, answer: rows([1n], [200n], { changed: true }) }], false, NATIVE));
  releaseCrop();
  await flush();
  page.frame();
  assert.equal(page.view(1).h, 10);
  assert.equal(page.widgets.get(1)!.installed!.c, 10, 'old pixels painted before crop must not be certified through hour 20');
});

test('S6: a malformed answer body inside a well-framed follow packet fails only its subscription, not the whole lane', async () => {
  const { page, lane, failures, released, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  mount(2, [{ id: 7, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.bindRoute(7, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [50n]) }, { id: 7, answer: rows([1n], [70n]) }], false, NATIVE));
  await step(10, { playing: false });
  const bad = message(20, [{ id: 5, answer: rows([1n], [55n]) }, { id: 7, answer: rows([1n], [75n]) }], false, NATIVE);
  bad[24 + 16] = 9;
  const releasedBefore = released.reduce((sum, bytes) => sum + bytes, 0);
  lane.receive('follow', bad);
  assert.ok(released.reduce((sum, bytes) => sum + bytes, 0) > releasedBefore, 'corrupt subscription record releases its charge immediately');
  await step(20, { playing: false });
  assert.deepEqual(failures, [], 'valid packet framing with one corrupt answer body must not kill the lane');
  assert.equal(page.view(1).mode, 'failed');
  assert.match(page.view(1).reason ?? '', /No followed answer kind 9/);
  assert.deepEqual([page.view(2).mode, valueOf(page.view(2), 7)], ['picture', 75n]);
});

test('S7: a failed panel decode stays sticky across STANDS, replacements, and seeks while stale-seek corrupt records never fault healthy subs', async () => {
  const { page, lane, failures, subFailures, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  mount(2, [{ id: 5, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: { shape: 'A' } }, { id: 5, hour: 10, kind: 'data', stepped: true, data: { shape: 'S' } }, { id: 0, hour: 10, kind: 'completed' }], 0));
  await step(10, { playing: false });
  assert.deepEqual([page.view(1).mode, page.view(2).mode], ['picture', 'picture']);
  const corrupt = panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: { shape: 'B' } }, { id: 0, hour: 20, kind: 'completed' }], 0);
  corrupt[24 + 16] = 0xc1;
  lane.receive('panels', corrupt);
  await step(20, { playing: false });
  assert.deepEqual(failures, []);
  assert.deepEqual(subFailures.map(([id]) => id), [3]);
  assert.equal(page.view(1).mode, 'failed');
  lane.receive('panels', panels([{ id: 3, hour: 25, kind: 'stands', stepped: true }, { id: 0, hour: 25, kind: 'completed' }], 0));
  await step(25, { playing: false });
  assert.equal(page.view(1).mode, 'failed', 'STANDS must not clear a panel decode fault or revert to the old panel');
  lane.receive('panels', panels([{ id: 3, hour: 30, kind: 'data', stepped: true, data: { shape: 'C' } }, { id: 0, hour: 30, kind: 'completed' }], 0));
  await step(30, { playing: false });
  assert.equal(page.view(1).mode, 'failed', 'a same-seek panel record must not clear a sticky panel decode fault');
  await step(40, { playing: false, seek: 1 });
  lane.receive('panels', panels([{ id: 3, hour: 40, kind: 'data', stepped: true, data: { shape: 'D' } }, { id: 5, hour: 40, kind: 'data', stepped: true, data: { shape: 'S1' } }, { id: 0, hour: 40, kind: 'completed' }], 1));
  await step(40, { playing: false });
  page.frame();
  assert.equal(page.view(1).mode, 'failed', 'sticky panel fault stays failed across seeks until re-subscription');
  assert.equal(page.view(2).mode, 'picture', 'healthy sibling panel keeps serving across seeks');
  const staleCorrupt = panels([{ id: 5, hour: 45, kind: 'data', stepped: true, data: { shape: 'Z' } }], 0);
  staleCorrupt[24 + 16] = 0xc1;
  lane.receive('panels', staleCorrupt);
  await step(40, { playing: false });
  assert.equal(page.view(2).mode, 'picture', 'a stale-seek corrupt panel must not fault a healthy subscription on the current seek');
});

test('P2/P3: a shared cell-group fault fails every owning subscription immediately, and a released cell never faults later', async () => {
  const CELL_A = 0x8000_0001, CELL_B = 0x8000_0002;
  const panelWith = (cell: number) => ({
    shape: `cell-${cell}`,
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: cell }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  });
  const { page, lane, subFailures, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  mount(2, [{ id: 5, channel: 'panel', product: 'demo' }]);
  mount(3, [{ id: 7, channel: 'panel', product: 'demo' }, { id: 9, channel: 'followed' }]);
  for (const sub of [3, 5, 7, 9]) lane.bindRoute(sub, ROUTE, NATIVE);
  lane.receive('panels', panels([
    { id: 3, hour: 10, kind: 'data', stepped: true, data: panelWith(CELL_A) },
    { id: 5, hour: 10, kind: 'data', stepped: true, data: panelWith(CELL_A) },
    { id: 7, hour: 10, kind: 'data', stepped: true, data: panelWith(CELL_A) },
    { id: 0, hour: 10, kind: 'completed' },
  ]));
  lane.receive('follow', message(10, [{ id: CELL_A, answer: rows([1n], [10n]) }], false, NATIVE));
  await step(10, { playing: false });
  assert.equal(page.view(1).mode, 'picture');
  assert.equal(page.view(2).mode, 'picture');
  assert.equal(page.view(3).mode, 'reading', 'widget 3 still waits on sub 9');
  lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: panelWith(CELL_B) }, { id: 0, hour: 20, kind: 'completed' }]));
  lane.receive('panels', panels([{ id: 3, hour: 30, kind: 'data', stepped: true, data: panelWith(CELL_B) }, { id: 0, hour: 30, kind: 'completed' }]));
  lane.receive('follow', message(30, [{ id: CELL_B, hour: 20, answer: rows([1n], [20n]) }], false, NATIVE));
  await step(30, { playing: false });
  await step(30, { playing: false });
  lane.receive('follow', message(35, [{ id: CELL_A, answer: rows([9n], [99n], { changed: true }) }, { id: CELL_B, answer: rows([1n], [35n], { changed: true }) }], false, NATIVE));
  lane.receive('panels', panels([{ id: 0, hour: 35, kind: 'completed' }]));
  await step(35, { playing: false });
  assert.deepEqual(subFailures.map(([id]) => id).sort((a, b) => a - b), [5, 7], 'cell fault fails every current owner sub (5 and 7), never retired sub 3');
  assert.equal(page.view(1).mode, 'picture', 'sub 3 released CELL_A after pruning and stays healthy on CELL_B');
  assert.equal(page.view(2).mode, 'failed');
  assert.equal(page.view(3).mode, 'failed', 'multi-sub widget 3 fails immediately on sub 7 fault even while sub 9 is pending');
});

test('S9: a subscription fault arriving while a normal picture is inflight is not lost after ack while paused', async () => {
  const { page, lane, mount } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [10n]) }], false, NATIVE));
  page.publish(10, { playing: false });
  await flush();
  await flush();
  lane.receive('follow', message(15, [{ id: 5, answer: rows([9n], [99n], { changed: true }) }], false, NATIVE));
  await flush();
  await flush();
  page.frame();
  await flush();
  await flush();
  page.frame();
  assert.equal(page.view(1).mode, 'failed', 'fault arriving while a normal picture was inflight must reach the page');
});

test('S10: a panel covering 10..30 with an unseeded cell group starting at 20 installs when target advances from 15 to 20', async () => {
  const CELL_A = 0x8000_0005;
  const panelData = {
    shape: 'unseeded',
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: CELL_A }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  };
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: panelData }, { id: 0, hour: 30, kind: 'completed' }]));
  lane.receive('follow', message(20, [{ id: CELL_A, answer: rows([1n], [200n]) }], false, NATIVE));
  await step(15, { playing: false });
  assert.equal(page.view(1).mode, 'reading');
  await step(20, { playing: false });
  assert.equal(page.view(1).mode, 'picture', 'lane revisits widget once target reaches cell group start hour 20');
});

test('S11: materializing an answer field collapses its patched worker state onto the compacted buffer', async () => {
  const { lane, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  const base = message(10, [{ id: 5, answer: { keys: [1n, 2n], columns: 1, values: [10n, 20n], names: ['A', 'B'] } }], false, NATIVE);
  lane.receive('follow', base);
  await step(10, { playing: false });
  const first = lane.facts.latest(5);
  assert.ok(first && first.kind === 'answer');
  const baseBuffer = first.answer.kind !== 'notRecorded' ? first.answer.views.u8.buffer : null;
  lane.receive('follow', message(20, [{ id: 5, answer: rows([1n], [99n], { changed: true }) }], false, NATIVE));
  await step(20, { playing: false });
  const latest = lane.facts.latest(5);
  assert.ok(latest && latest.kind === 'answer' && latest.answer.kind !== 'notRecorded');
  assert.equal(latest.answer.patches, undefined, 'materialized answer collapses patched tree');
  assert.notEqual(latest.answer.views.u8.buffer, baseBuffer, 'materialized answer drops reference to the old base buffer');
});

test('S14: a cheap data widget changing every frame does not starve an oversized motion widget', async () => {
  let clock = 0;
  let motionPaints = 0;
  let cheapPaints = 0;
  const views: Record<string, LaneView> = {
    heavyMotion: {
      paint: (input): Painted => {
        clock += 15;
        motionPaints++;
        return { surfaces: [], fields: { f: { value: motionPaints, content: `m:${motionPaints}:${input.now}` } }, motion: true };
      },
    },
    cheapData: {
      paint: (input): Painted => {
        clock += 0.2;
        cheapPaints++;
        return { surfaces: [], fields: { h: { value: input.pictures[0]!.h, content: `c:${input.pictures[0]!.h}` } }, motion: false };
      },
    },
  };
  const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => { if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target); } });
  const lane = new Lane<Fake>({
    lane: 0, lanes: 1, views: kind => views[kind]!, crop: async () => [],
    present: presentation => page.offer(presentation), release: () => {}, fail: () => {}, later: () => {},
    budget: 10, clock: () => clock,
  });
  lane.follow(3, 'followed'); lane.bindRoute(3, ROUTE, NATIVE);
  lane.mount(1, [3], 'heavyMotion'); lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
  page.mount(1, { route: ROUTE, lane: 0, surfaces: [] });
  lane.follow(5, 'followed'); lane.bindRoute(5, ROUTE, NATIVE);
  lane.mount(2, [5], 'cheapData'); lane.bind(2, { generation: 1, surfaces: [], offset: 0, pin: null });
  page.mount(2, { route: ROUTE, lane: 0, surfaces: [] });
  lane.receive('follow', message(10, [{ id: 3, answer: rows([1n], [1n]) }, { id: 5, answer: rows([1n], [10n]) }], false, NATIVE));
  page.publish(10, { playing: true, step: 1, speed: 24 }); await flush(); await flush(); page.frame(); await flush();
  let paintsAfterWarmup = 0;
  for (let frame = 1; frame <= 20; frame++) {
    if (frame === 7) paintsAfterWarmup = motionPaints;
    lane.receive('follow', message(10 + frame, [{ id: 5, answer: rows([1n], [BigInt(10 + frame)], { changed: true }) }], false, NATIVE));
    page.publish(10 + frame, { playing: true, step: 1, speed: 24 }); await flush(); await flush(); page.frame(); await flush();
  }
  assert.ok(motionPaints - paintsAfterWarmup >= 3, `oversized motion widget starved after cost > budget: only ${motionPaints - paintsAfterWarmup} paints in 14 frames`);
  assert.ok(cheapPaints >= 14, `cheap widget painted ${cheapPaints} times`);
});

test('S7b: COMPLETED never extends pre-fault panel data across a decode fault, and the fault stays sticky across seeks', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: { shape: 'H10' } }, { id: 0, hour: 10, kind: 'completed' }], 0));
  await step(10, { playing: false });
  assert.equal(page.view(1).mode, 'picture');
  const corrupt = panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: { shape: 'H20' } }, { id: 0, hour: 20, kind: 'completed' }], 0);
  corrupt[24 + 16] = 0xc1;
  lane.receive('panels', corrupt);
  lane.receive('panels', panels([{ id: 0, hour: 25, kind: 'completed' }], 0));
  lane.receive('panels', panels([{ id: 3, hour: 30, kind: 'data', stepped: true, data: { shape: 'H30' } }, { id: 0, hour: 30, kind: 'completed' }], 0));
  await step(25, { playing: false });
  page.frame();
  assert.equal(page.view(1).mode, 'failed', 'paused at 25 must show the sticky decode fault, never hour-10 data certified through 25');
  await step(24, { playing: false, seek: 1 });
  assert.equal(page.view(1).mode, 'failed', 'seeking to 24 keeps the sticky subscription failure and never carries pre-fault data');
});

test('S12b: a lane widget outside its active window does not force O(N) scans of all windowed widgets on quiet frames', async () => {
  const { lane, mount, step } = rig({}, { clock: () => 0 });
  mount(1, [{ id: 3, channel: 'followed' }]);
  lane.bindRoute(3, ROUTE, NATIVE);
  for (let i = 2; i <= 60; i++) {
    const sub = i * 2 + 1;
    mount(i, [{ id: sub, channel: 'followed' }]);
    lane.bindRoute(sub, ROUTE, NATIVE);
  }
  lane.receive('follow', message(10, Array.from({ length: 60 }, (_, idx) => ({ id: (idx + 1) * 2 + 1, answer: rows([1n], [BigInt(idx + 1)]) })), false, NATIVE));
  lane.receive('follow', message(20, [{ id: 3, answer: rows([1n], [999n], { changed: true }) }], false, NATIVE));
  await step(15, { playing: false });
  await step(50, { playing: true, seek: 1, step: 24, speed: 1440 });
  lane.receive('follow', message(50, Array.from({ length: 59 }, (_, idx) => ({ id: (idx + 2) * 2 + 1, answer: rows([1n], [BigInt(idx + 2)]) })), true, NATIVE, 1));
  await step(50, { playing: true, step: 24, speed: 1440 });
  await step(74, { playing: true, step: 24, speed: 1440 });
  const before = lane.stats.visited;
  for (let h = 98; h <= 200; h += 24) {
    await step(h, { playing: true, step: 24, speed: 1440 });
  }
  assert.equal(lane.stats.visited - before, 0, `quiet frames visited ${lane.stats.visited - before} widgets`);
});

test('S14b: four always-changing 6 ms data widgets do not starve one 15 ms motion widget on a 10 ms budget', async () => {
  let clock = 0;
  let motionPaints = 0;
  const views: Record<string, LaneView> = {
    motion15: {
      paint: (input): Painted => {
        clock += 15;
        motionPaints++;
        return { surfaces: [], fields: { f: { value: motionPaints, content: `m:${motionPaints}:${input.now}` } }, motion: true };
      },
    },
    data6: {
      paint: (input): Painted => {
        clock += 6;
        return { surfaces: [], fields: { h: { value: input.pictures[0]!.h, content: `d:${input.pictures[0]!.h}` } }, motion: false };
      },
    },
  };
  const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => { if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target); } });
  const lane = new Lane<Fake>({
    lane: 0, lanes: 1, views: kind => views[kind]!, crop: async () => [],
    present: presentation => page.offer(presentation), release: () => {}, fail: () => {}, later: () => {},
    budget: 10, clock: () => clock,
  });
  lane.follow(3, 'followed'); lane.bindRoute(3, ROUTE, NATIVE);
  lane.mount(1, [3], 'motion15'); lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
  page.mount(1, { route: ROUTE, lane: 0, surfaces: [] });
  for (let id = 2; id <= 5; id++) {
    const sub = id * 2 + 1;
    lane.follow(sub, 'followed'); lane.bindRoute(sub, ROUTE, NATIVE);
    lane.mount(id, [sub], 'data6'); lane.bind(id, { generation: 1, surfaces: [], offset: 0, pin: null });
    page.mount(id, { route: ROUTE, lane: 0, surfaces: [] });
  }
  lane.receive('follow', message(10, [3, 5, 7, 9, 11].map(id => ({ id, answer: rows([1n], [10n]) })), false, NATIVE));
  page.publish(10, { playing: true, step: 1, speed: 24 }); await flush(); await flush();
  let at30 = 0;
  for (let frame = 1; frame <= 120; frame++) {
    if (frame === 30) at30 = motionPaints;
    lane.receive('follow', message(10 + frame, [5, 7, 9, 11].map(id => ({ id, answer: rows([1n], [BigInt(10 + frame)], { changed: true }) })), false, NATIVE));
    page.publish(10 + frame, { playing: true, step: 1, speed: 24 }); await flush(); await flush();
  }
  assert.ok(motionPaints > at30 + 5, `motion widget starved after frame 30: ${at30} at frame 30, ${motionPaints} at frame 120`);
});

test('N2: on seek, early followed cell answer and early panel naming that cell both drain immediately while paused', async () => {
  const CELL_X = 0x8000_0021;
  const panelData = {
    shape: 'early-cell',
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: CELL_X }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  };
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  await step(10, { playing: false, seek: 0 });
  lane.receive('follow', message(50, [{ id: CELL_X, answer: rows([1n], [777n]) }], true, NATIVE, 1));
  lane.receive('panels', panels([{ id: 3, hour: 50, kind: 'data', stepped: true, data: panelData }, { id: 0, hour: 50, kind: 'completed' }], 1));
  await step(50, { playing: false, seek: 1 });
  page.frame();
  assert.equal(page.view(1).mode, 'picture', 'paused widget must install without waiting for an unrelated later packet');
});

test('N3: rewinding into carried panel A keeps latest native panel B cell ownership until new-seek panel replaces it', async () => {
  const CELL_A = 0x8000_0031, CELL_B = 0x8000_0032;
  const panelWith = (cell: number, shape: string) => ({
    shape,
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: cell }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  });
  const { page, lane, failures, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: panelWith(CELL_A, 'A') }, { id: 0, hour: 20, kind: 'completed' }], 0));
  lane.receive('follow', message(10, [{ id: CELL_A, answer: rows([1n], [100n]) }], false, NATIVE, 0));
  lane.receive('follow', message(20, [], false, NATIVE, 0));
  await step(15, { playing: false });
  lane.receive('panels', panels([{ id: 3, hour: 30, kind: 'data', stepped: true, data: panelWith(CELL_B, 'B') }, { id: 0, hour: 35, kind: 'completed' }], 0));
  lane.receive('follow', message(35, [{ id: CELL_B, answer: rows([1n], [200n]) }], false, NATIVE, 0));
  await step(15, { playing: false, seek: 1 });
  lane.receive('panels', panels([{ id: 0, hour: 15, kind: 'completed' }], 1));
  lane.receive('follow', message(15, [{ id: CELL_B, answer: rows([1n], [250n]) }], true, NATIVE, 1));
  lane.receive('panels', panels([{ id: 3, hour: 15, kind: 'data', stepped: true, data: panelWith(CELL_B, 'B1') }, { id: 0, hour: 16, kind: 'completed' }], 1));
  lane.receive('follow', message(16, [{ id: CELL_B, answer: rows([1n], [260n], { changed: true }) }], false, NATIVE, 1));
  await step(16, { playing: false });
  page.frame();
  assert.deepEqual(failures, []);
  assert.equal(page.view(1).mode, 'picture', 'delta on reused cell B after rewind into carried panel A must not fail with missing base');
});

test('N4: a rejected crop from seek 0 arriving after seek 1 never installs a failure under seek 1', async () => {
  let rejectCrop!: (err: Error) => void;
  let calls = 0;
  const views: Record<string, LaneView> = {
    canvas: {
      paint: (input): Painted => ({
        surfaces: [{ source: { width: 1, height: 1, content: `h:${input.pictures[0]!.h}` } as unknown as { width: number; height: number }, content: `h:${input.pictures[0]!.h}` }],
        fields: {},
        motion: false,
      }),
    },
  };
  const { page, lane, mount, step } = rig(views, {
    crop: sources => {
      calls++;
      if (calls === 1) return new Promise((_resolve, reject) => { rejectCrop = reject; });
      return Promise.resolve(sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: 1, close() { this.closed = true; } })));
    },
  });
  mount(1, [{ id: 3, channel: 'followed' }], 'canvas', 1);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  lane.receive('follow', message(10, [{ id: 3, answer: rows([1n], [10n]) }], false, NATIVE, 0));
  page.publish(10, { playing: false, seek: 0 });
  await flush();
  page.publish(55, { playing: false, seek: 1 });
  await flush();
  rejectCrop(new Error('GPU lost on old seek'));
  await flush(); await flush(); page.frame();
  assert.notEqual(page.view(1).mode, 'failed', 'seek 0 crop rejection must not fail seek 1');
  lane.receive('follow', message(55, [{ id: 3, answer: rows([1n], [55n]) }], true, NATIVE, 1));
  await step(55, { playing: false });
  page.frame();
  assert.equal(page.view(1).mode, 'picture');
  assert.equal(page.view(1).h, 55);
});

test('N6: future-seek completion-only panel packets coalesce and future-seek queue overflow produces a named failure', async () => {
  const { page, lane, mount, step } = rig({}, { earlyBuffer: 512 });
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  await step(10, { playing: false, seek: 0 });
  for (let h = 1; h <= 200; h++) {
    lane.receive('panels', panels([{ id: 0, hour: h, kind: 'completed' }], 1));
  }
  lane.receive('panels', panels([{ id: 3, hour: 100, kind: 'data', stepped: true, data: { shape: 'ok' } }, { id: 0, hour: 200, kind: 'completed' }], 1));
  await step(100, { playing: false, seek: 1 });
  page.frame();
  assert.equal(page.view(1).mode, 'picture', '200 redundant future-seek completion packets coalesce without overflowing');
  for (let i = 0; i < 10; i++) {
    lane.receive('panels', panels([{ id: 3, hour: 300 + i, kind: 'data', stepped: true, data: { shape: 'big', pad: 'x'.repeat(120) } }], 2));
  }
  await step(300, { playing: false, seek: 2 });
  page.frame();
  assert.equal(page.view(1).mode, 'failed', 'future-seek panel queue overflow reports a named failure');
  assert.match(page.view(1).reason ?? '', /overflowed/i);
});

test('N7: completion-only panel packets on quiet frames visit zero unchanged stepped panel widgets', async () => {
  const { page, lane, mount, step } = rig({}, { clock: () => 0 });
  const initial: PanelRow[] = [];
  for (let i = 1; i <= 50; i++) {
    const sub = i * 2 + 1;
    mount(i, [{ id: sub, channel: 'panel', product: 'demo' }]);
    lane.bindRoute(sub, ROUTE, NATIVE);
    initial.push({ id: sub, hour: 10, kind: 'data', stepped: true, data: { shape: `S${i}` } });
  }
  initial.push({ id: 0, hour: 10, kind: 'completed' });
  lane.receive('panels', panels(initial, 0));
  await step(10, { playing: true, step: 24, speed: 1440 });
  await step(34, { playing: true, step: 24, speed: 1440 });
  const beforeVisited = lane.stats.visited;
  for (let h = 58; h <= 202; h += 24) {
    lane.receive('panels', panels([{ id: 0, hour: h, kind: 'completed' }], 0));
    await step(h, { playing: true, step: 24, speed: 1440 });
  }
  assert.equal(lane.stats.visited - beforeVisited, 0, `completion-only panel packets visited ${lane.stats.visited - beforeVisited} unchanged panel widgets`);
  await step(200, { playing: false });
  assert.equal(page.view(1).dim, false, 'shared panel confirmation keeps stepped panels undimmed on pause');
});

test('M1: corrupt followed delta followed by quiet update, non-reset replacement, and seek stays failed and never carries pre-fault values', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [100n]) }], false, NATIVE, 0));
  await step(10, { playing: false });
  assert.deepEqual([page.view(1).mode, valueOf(page.view(1), 5)], ['picture', 100n]);
  lane.receive('follow', message(20, [{ id: 5, answer: rows([9n], [999n], { changed: true }) }], false, NATIVE, 0));
  lane.receive('follow', message(25, [], false, NATIVE, 0));
  lane.receive('follow', message(30, [{ id: 5, answer: rows([1n], [300n]) }], false, NATIVE, 0));
  await step(25, { playing: false });
  assert.equal(page.view(1).mode, 'failed', 'corrupt delta is sticky; quiet update and same-seek replacement must not clear it');
  await step(30, { playing: false });
  assert.equal(page.view(1).mode, 'failed', 'same-seek full replacement must not clear sticky merge fault');
  await step(25, { playing: false, seek: 1 });
  assert.equal(page.view(1).mode, 'failed', 'seeking after fault keeps sticky failure and never carries pre-fault hour-10 values');
});

test('M2: future-seek panel replay preserves completion order relative to data records while coalescing adjacent completions', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  await step(5, { playing: false, seek: 0 });
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: { shape: 'A' } }, { id: 0, hour: 10, kind: 'completed' }], 1));
  lane.receive('panels', panels([{ id: 0, hour: 15, kind: 'completed' }], 1));
  lane.receive('panels', panels([{ id: 0, hour: 19, kind: 'completed' }], 1));
  lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: { shape: 'B' } }, { id: 0, hour: 20, kind: 'completed' }], 1));
  await step(19, { playing: false, seek: 1 });
  page.frame();
  const view19 = page.view(1);
  assert.deepEqual([view19.mode, view19.h, view19.through, view19.dim, (view19.fields?.['model:3'] as { structure?: string }).structure],
    ['picture', 10, 19, false, 'A'], 'A must keep coverage through 19 instead of moving completion 19 past B at 20');
  await step(20, { playing: false });
  const view20 = page.view(1);
  assert.deepEqual([view20.mode, view20.h, view20.through, (view20.fields?.['model:3'] as { structure?: string }).structure],
    ['picture', 20, 20, 'B']);
});

test('M6: a pinned animated widget never invalidates finite-window safe bounds for other widgets', async () => {
  let frames = 0;
  const spin: LaneView = { paint: (): Painted => ({ surfaces: [], fields: { f: { value: ++frames, content: `f:${frames}` } }, motion: true }) };
  const { lane, mount, step } = rig({ spin }, { clock: () => 0 });
  mount(1, [{ id: 3, channel: 'followed' }], 'spin', 0, { pin: 10 });
  lane.bindRoute(3, ROUTE, NATIVE);
  const subs: { id: number; answer: ReturnType<typeof rows> }[] = [{ id: 3, answer: rows([1n], [1n]) }];
  for (let i = 2; i <= 61; i++) {
    const sub = i * 2 + 1;
    mount(i, [{ id: sub, channel: 'followed' }]);
    lane.bindRoute(sub, ROUTE, NATIVE);
    subs.push({ id: sub, answer: rows([1n], [BigInt(i)]) });
  }
  lane.receive('follow', message(10, subs, false, NATIVE));
  await step(10, { playing: true, step: 24, speed: 1440 });
  await step(34, { playing: true, step: 24, speed: 1440 });
  const before = lane.stats.visited;
  for (let h = 58; h <= 154; h += 24) {
    await step(h, { playing: true, step: 24, speed: 1440 });
  }
  assert.ok(lane.stats.visited - before <= 10, `pinned animated widget triggered ${lane.stats.visited - before} visits across 60 finite-window widgets`);
});

test('N6b: future-seek overflow tracking stays O(1) bounded across many successive overflowed future seeks', async () => {
  const { page, lane, mount, step } = rig({}, { earlyBuffer: 256 });
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  await step(10, { playing: false, seek: 0 });
  for (let s = 1; s <= 100; s++) {
    for (let i = 0; i < 4; i++) {
      lane.receive('panels', panels([{ id: 3, hour: 100 + i, kind: 'data', stepped: true, data: { shape: 'big', pad: 'x'.repeat(120) } }], s));
    }
  }
  const map = (lane as unknown as { earlyOverflow: Map<number, unknown> }).earlyOverflow;
  const entry = map.get(NATIVE);
  assert.equal(typeof entry, 'number', 'earlyOverflow must store only the newest overflowed future seek per route, not an unbounded Set');
  assert.equal(entry, 100);
  await step(100, { playing: false, seek: 100 });
  page.frame();
  assert.equal(page.view(1).mode, 'failed');
  assert.match(page.view(1).reason ?? '', /overflowed/i);
});

test('N7b: completion-only panel packets skip non-stepped panels and do zero fact-store subscription scans', async () => {
  const { page, lane, mount, step } = rig({}, { clock: () => 0 });
  const initial: PanelRow[] = [];
  for (let i = 1; i <= 30; i++) {
    const sub = i * 2 + 1;
    mount(i, [{ id: sub, channel: 'panel', product: 'demo' }]);
    lane.bindRoute(sub, ROUTE, NATIVE);
    initial.push({ id: sub, hour: 10, kind: 'data', stepped: i <= 15, data: { shape: `P${i}` } });
  }
  initial.push({ id: 0, hour: 10, kind: 'completed' });
  lane.receive('panels', panels(initial, 0));
  await step(10, { playing: true, step: 24, speed: 1440 });
  await step(34, { playing: true, step: 24, speed: 1440 });
  const beforeVisited = lane.stats.visited;
  const beforeCompletionVisits = (lane.facts as unknown as { stats?: { completionVisits: number } }).stats?.completionVisits ?? -1;
  for (let h = 58; h <= 178; h += 24) {
    lane.receive('panels', panels([{ id: 0, hour: h, kind: 'completed' }], 0));
    await step(h, { playing: true, step: 24, speed: 1440 });
  }
  assert.equal(lane.stats.visited - beforeVisited, 0, `visited ${lane.stats.visited - beforeVisited} panel widgets (including non-stepped)`);
  assert.equal(((lane.facts as unknown as { stats?: { completionVisits: number } }).stats?.completionVisits ?? -1) - beforeCompletionVisits, 0, 'fact store scanned subscriptions on monotonic COMPLETED');
  await step(178, { playing: false });
  assert.equal(page.view(1).dim, false, 'stepped panel stays confirmed through 178');
  assert.equal(page.view(20).h, 10, 'non-stepped panel stays intact');
});

test('P8: replacement stepped panel followed by equal completion-only packet replaces paused panel A undimmed at 50', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: { shape: 'A' } }, { id: 0, hour: 50, kind: 'completed' }], 0));
  await step(50, { playing: false });
  assert.deepEqual([page.view(1).mode, page.view(1).h, page.view(1).dim, (page.view(1).fields?.['model:3'] as { structure?: string }).structure], ['picture', 10, false, 'A']);
  lane.receive('panels', panels([{ id: 3, hour: 40, kind: 'data', stepped: true, data: { shape: 'B' } }], 0));
  await step(50, { playing: false });
  lane.receive('panels', panels([{ id: 0, hour: 50, kind: 'completed' }], 0));
  await step(50, { playing: false });
  page.frame();
  assert.deepEqual([page.view(1).mode, page.view(1).h, page.view(1).dim, (page.view(1).fields?.['model:3'] as { structure?: string }).structure], ['picture', 40, false, 'B']);
});

test('P9: overflowing seeks 1 and 2 before entering 1 drops seek 1 silently and fails on entering seek 2', async () => {
  const { page, lane, subFailures, mount, step } = rig({}, { earlyBuffer: 256 });
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  await step(10, { playing: false, seek: 0 });
  for (let i = 0; i < 4; i++) lane.receive('panels', panels([{ id: 3, hour: 100 + i, kind: 'data', stepped: true, data: { shape: 'big1', pad: 'x'.repeat(120) } }], 1));
  for (let i = 0; i < 4; i++) lane.receive('panels', panels([{ id: 3, hour: 200 + i, kind: 'data', stepped: true, data: { shape: 'big2', pad: 'x'.repeat(120) } }], 2));
  await step(100, { playing: false, seek: 1 });
  assert.deepEqual(subFailures, [], 'superseded seek 1 must pass through silently without failing');
  assert.notEqual(page.view(1).mode, 'failed');
  await step(200, { playing: false, seek: 2 });
  assert.equal(subFailures.length, 1, 'entering overflowed newest future seek 2 fails the subscription once');
  assert.equal(subFailures[0]![0], 3);
  assert.equal(page.view(1).mode, 'failed');
  assert.match(page.view(1).reason ?? '', /overflowed/i);
});

test('P10: malformed live section or group in panel data fails the subscription instead of being swallowed', () => {
  const { page, lane, subFailures, mount } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: { live: { groups: 'not-an-array', rows: [], kinds: [], cells: [] } } }], 0));
  assert.equal(subFailures.length, 1);
  assert.equal(subFailures[0]![0], 3);
  assert.equal(page.view(1).mode, 'failed');
});

test('P6: cell-group fault fails owner subscriptions only, never keyed by cell id so a healthy subscription reusing that cell id succeeds', async () => {
  const CELL_A = 0x8000_0077;
  const panelData = {
    shape: 'reuse',
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: CELL_A }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  };
  const { page, lane, subFailures, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: panelData }, { id: 0, hour: 10, kind: 'completed' }], 0));
  lane.receive('follow', message(10, [{ id: CELL_A, answer: rows([1n], [10n]) }], false, NATIVE, 0));
  lane.receive('follow', message(20, [{ id: CELL_A, answer: rows([9n], [99n], { changed: true }) }], false, NATIVE, 0));
  assert.deepEqual(subFailures.map(([id]) => id), [3], 'fault reported against owner subscription 3, never cell id');
  assert.equal(page.view(1).mode, 'failed');
  mount(2, [{ id: 5, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(5, ROUTE, NATIVE, 0);
  lane.receive('panels', panels([{ id: 5, hour: 30, kind: 'data', stepped: true, data: panelData }, { id: 0, hour: 30, kind: 'completed' }], 0));
  lane.receive('follow', message(30, [{ id: CELL_A, answer: rows([1n], [300n]) }], false, NATIVE, 0));
  await step(30, { playing: false, seek: 0 });
  assert.equal(page.view(2).mode, 'picture', 'healthy subscription 5 reusing CELL_A must not inherit sub 3 fault');
  assert.equal((page.view(2).fields?.['live:5'] as Map<string, unknown> | undefined)?.get('r0\u0000k'), 300);
});

const followCharge = (bytes: Uint8Array) => {
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let total = 0;
  for (let i = 0, at = 24, count = data.getUint32(12, true); i < count; i++) {
    const length = data.getUint32(at + 8, true);
    const uncompressed = data.getUint32(at + 12, true);
    total += 16 + 2 * Math.max(length, uncompressed);
    at += 16 + length;
  }
  return total;
};

const panelCharge = (bytes: Uint8Array) => {
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let total = 0;
  for (let i = 0, at = 24, count = data.getUint32(8, true); i < count; i++) {
    const kind = data.getUint8(at + 8), length = data.getUint32(at + 12, true), size = ((16 + length + 7) & ~7);
    if (kind !== 4) total += 2 * size;
    at += size;
  }
  return total;
};

test('M1/F1: same-packet valid record followed by corrupt record commits nothing past the fault and returns full credit on live and held paths', async () => {
  const { page, lane, released, subFailures, mount, step } = rig();
  const totalReleased = () => released.reduce((sum, b) => sum + b, 0);
  let totalCharged = 0;
  const sendFollow = (pkt: Uint8Array) => { totalCharged += followCharge(pkt); lane.receive('follow', pkt); };
  const sendPanels = (pkt: Uint8Array) => { totalCharged += panelCharge(pkt); lane.receive('panels', pkt); };

  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE, 0);
  sendFollow(message(10, [
    { id: 5, hour: 10, answer: rows([1n], [100n]) },
    { id: 5, hour: 10, answer: rows([9n], [999n], { changed: true }) },
  ], false, NATIVE, 0));
  assert.equal(subFailures.length, 1);
  assert.equal(page.view(1).mode, 'failed');
  assert.equal(lane.facts.select(5, 10), null, 'M1: valid table before corrupt delta in same packet must not survive failSub');
  assert.equal(totalReleased() + lane.retainedBytes(), totalCharged);
  await step(10, { playing: false, seek: 1 });
  assert.equal(lane.facts.select(5, 10), null, 'M1: seek must never carry pre-fault table from same packet');
  assert.equal(page.view(1).mode, 'failed');

  mount(2, [{ id: 7, channel: 'followed' }]);
  const heldBefore = totalReleased();
  const heldBatch = message(10, [
    { id: 7, hour: 10, answer: rows([1n], [100n]) },
    { id: 7, hour: 10, answer: rows([9n], [999n], { changed: true }) },
  ], false, NATIVE, 1);
  sendFollow(heldBatch);
  lane.bindRoute(7, ROUTE, NATIVE, 1);
  assert.equal(page.view(2).mode, 'failed');
  assert.equal(lane.facts.select(7, 10), null, 'M1 held: valid table before corrupt delta must not survive replay fault');
  assert.equal(totalReleased() - heldBefore, followCharge(heldBatch), 'M1 held: all charged bytes from both records must return');
  assert.equal(totalReleased() + lane.retainedBytes(), totalCharged);

  const CELL_A = 0x8000_0091;
  const panelWith = (shape: string) => ({
    shape,
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: CELL_A }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  });
  mount(3, [{ id: 9, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(9, ROUTE, NATIVE, 1);
  const badPanelBatch = panels([
    { id: 9, hour: 10, kind: 'data', stepped: true, data: panelWith('H10') },
    { id: 9, hour: 10, kind: 'data', stepped: true, data: { live: { groups: 'bad', rows: [], kinds: [], cells: [] } } },
    { id: 0, hour: 10, kind: 'completed' },
  ], 1);
  sendPanels(badPanelBatch);
  assert.equal(page.view(3).mode, 'failed');
  assert.equal(lane.facts.owns(CELL_A), false, 'F1: malformed second panel in same packet must not leave CELL_A owned by failed sub');
  assert.equal(lane.facts.select(9, 10), null);
  assert.equal(totalReleased() + lane.retainedBytes(), totalCharged);

  mount(4, [{ id: 11, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(11, ROUTE, NATIVE, 1);
  sendPanels(panels([{ id: 11, hour: 20, kind: 'data', stepped: true, data: panelWith('H20') }, { id: 0, hour: 20, kind: 'completed' }], 1));
  sendFollow(message(20, [{ id: CELL_A, hour: 20, answer: rows([1n], [222n]) }], false, NATIVE, 1));
  await step(20, { playing: false });
  assert.equal(page.view(4).mode, 'picture');
  assert.equal((page.view(4).fields?.['live:11'] as Map<string, unknown> | undefined)?.get('r0\u0000k'), 222);
  assert.equal(totalReleased() + lane.retainedBytes(), totalCharged);
});

test('credit invariant: charged bytes always equal retained plus released across live, held, early, overflow, fault, cell reuse, seek, and unfollow sequences', async () => {
  const { lane, released, mount, step } = rig({}, { earlyBuffer: 320 });
  const totalReleased = () => released.reduce((sum, b) => sum + b, 0);
  let totalCharged = 0;
  const check = (label: string) => assert.equal(totalReleased() + lane.retainedBytes(), totalCharged, label);
  const sendFollow = (pkt: Uint8Array, label: string) => { totalCharged += followCharge(pkt); lane.receive('follow', pkt); check(label); };
  const sendPanels = (pkt: Uint8Array, label: string) => { totalCharged += panelCharge(pkt); lane.receive('panels', pkt); check(label); };

  const CELL_A = 0x8000_00a1;
  const panelData = (tag: string) => ({
    shape: tag,
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: CELL_A }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  });
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  mount(2, [{ id: 5, channel: 'followed' }]);
  sendFollow(message(5, [{ id: 5, answer: rows([1n], [1n]) }], false, NATIVE, 0), 'unbound follow seek 0');
  sendFollow(message(6, [{ id: 5, answer: rows([1n], [2n]) }], false, NATIVE, 1), 'unbound follow supersedes seek 0');
  lane.bindRoute(3, ROUTE, NATIVE, 1); check('bind sub 3');
  lane.bindRoute(5, ROUTE, NATIVE, 1); check('bind sub 5');
  sendPanels(panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: panelData('A') }, { id: 0, hour: 10, kind: 'completed' }], 1), 'live panel');
  sendFollow(message(10, [{ id: CELL_A, answer: rows([1n], [10n]) }], false, NATIVE, 1), 'live cell whole');
  await step(10, { playing: false, seek: 1 }); check('step 10');
  sendFollow(message(11, [{ id: CELL_A, answer: rows([9n], [99n], { changed: true }) }], false, NATIVE, 1), 'cell fault');
  sendFollow(message(12, [{ id: CELL_A, answer: rows([1n], [12n], { changed: true }) }], false, NATIVE, 1), 'stale delta after fault');
  sendFollow(message(13, [{ id: CELL_A, answer: rows([1n], [13n]) }], false, NATIVE, 1), 'fresh whole for reused cell');
  for (let i = 0; i < 6; i++) {
    sendPanels(panels([{ id: 3, hour: 20 + i, kind: 'data', stepped: true, data: { shape: 'big', pad: 'x'.repeat(120) } }], 2), `early overflow ${i}`);
  }
  await step(20, { playing: false, seek: 2 }); check('seek 2 after overflow');
  sendFollow(message(19, [{ id: 5, answer: rows([1n], [19n]) }], false, NATIVE, 1), 'stale seek follow');
  lane.unfollow(3); check('unfollow 3');
  lane.unfollow(5); check('unfollow 5');
  lane.unmount(1); lane.unmount(2);
  await step(20, { playing: false, seek: 2 }); check('after unmount');
  assert.equal(lane.retainedBytes(), 0, 'all bytes returned once all subs and widgets are gone');
});

test('P6/F3: fresh whole answer arriving before a healthy panel reuses a faulted cell id is held and merges later deltas', async () => {
  const CELL_A = 0x8000_00b1;
  const panelData = (tag: string) => ({
    shape: tag,
    live: { groups: [{ table: 1, columns: [0], keys: ['1'], follow: CELL_A }], rows: ['r0'], kinds: [{ id: 'k', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] },
  });
  const { page, lane, subFailures, mount, step } = rig();
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: panelData('P1') }, { id: 0, hour: 10, kind: 'completed' }], 0));
  lane.receive('follow', message(10, [{ id: CELL_A, answer: rows([1n], [10n]) }], false, NATIVE, 0));
  lane.receive('follow', message(11, [{ id: CELL_A, answer: rows([9n], [99n], { changed: true }) }], false, NATIVE, 0));
  assert.deepEqual(subFailures.map(([id]) => id), [3]);
  assert.equal(page.view(1).mode, 'failed');

  lane.receive('follow', message(12, [{ id: CELL_A, answer: rows([1n], [12n], { changed: true }) }], false, NATIVE, 0));
  lane.receive('follow', message(20, [{ id: CELL_A, answer: rows([1n], [200n]) }], false, NATIVE, 0));
  mount(2, [{ id: 5, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(5, ROUTE, NATIVE, 0);
  lane.receive('panels', panels([{ id: 5, hour: 20, kind: 'data', stepped: true, data: panelData('P2') }, { id: 0, hour: 20, kind: 'completed' }], 0));
  await step(20, { playing: false });
  assert.equal(page.view(2).mode, 'picture', 'healthy panel naming reused CELL_A after fresh whole answer must install, never stay on Reading');
  assert.equal((page.view(2).fields?.['live:5'] as Map<string, unknown> | undefined)?.get('r0\u0000k'), 200);

  lane.receive('follow', message(21, [{ id: CELL_A, answer: rows([1n], [205n], { changed: true }) }], false, NATIVE, 0));
  lane.receive('panels', panels([{ id: 0, hour: 21, kind: 'completed' }], 0));
  await step(21, { playing: false });
  assert.equal(page.view(2).mode, 'picture', 'subsequent delta merges onto the held whole base cleanly');
  assert.equal((page.view(2).fields?.['live:5'] as Map<string, unknown> | undefined)?.get('r0\u0000k'), 205);
});

test('P9/F4: future-seek overflow delivers failure to an unbound subscription when it later binds at that seek', async () => {
  const { page, lane, subFailures, mount, step } = rig({}, { earlyBuffer: 256 });
  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  mount(2, [{ id: 5, channel: 'panel', product: 'demo' }]);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  await step(10, { playing: false, seek: 0 });
  for (let i = 0; i < 4; i++) {
    lane.receive('panels', panels([
      { id: 3, hour: 100 + i, kind: 'data', stepped: true, data: { shape: 'A', pad: 'x'.repeat(100) } },
      { id: 5, hour: 100 + i, kind: 'data', stepped: true, data: { shape: 'B', pad: 'x'.repeat(100) } },
    ], 1));
  }
  await step(100, { playing: false, seek: 1 });
  assert.deepEqual(subFailures.map(([id]) => id), [3], 'bound sub A fails on entering seek 1');
  assert.equal(page.view(1).mode, 'failed');

  lane.bindRoute(5, ROUTE, NATIVE, 1);
  lane.receive('panels', panels([{ id: 5, hour: 101, kind: 'stands', stepped: true }, { id: 0, hour: 101, kind: 'completed' }], 1));
  await step(101, { playing: false, seek: 1 });
  assert.deepEqual(subFailures.map(([id]) => id), [3, 5], 'unbound sub B receives overflow failure when it binds at seek 1');
  assert.equal(page.view(2).mode, 'failed');
  assert.match(page.view(2).reason ?? '', /overflowed/i);
});

test('F5: unbound native route drops superseded seek-1 malformed panel when seek-2 healthy panel arrives before bindRoute', async () => {
  const { page, lane, subFailures, released, mount, step } = rig();
  const totalReleased = () => released.reduce((sum, b) => sum + b, 0);
  let totalCharged = 0;
  const sendPanels = (pkt: Uint8Array) => { totalCharged += panelCharge(pkt); lane.receive('panels', pkt); };

  mount(1, [{ id: 3, channel: 'panel', product: 'demo' }]);
  const badSeek1 = panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: { shape: 'bad' } }], 1);
  badSeek1[24 + 16] = 0xc1;
  sendPanels(badSeek1);
  sendPanels(panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: { shape: 'healthy2' } }, { id: 0, hour: 20, kind: 'completed' }], 2));
  assert.equal(totalReleased(), panelCharge(badSeek1), 'superseded seek-1 malformed panel is released before bind');

  lane.bindRoute(3, ROUTE, NATIVE, 1);
  assert.deepEqual(subFailures, [], 'binding at seek 1 must not decode superseded seek-1 malformed panel');
  assert.notEqual(page.view(1).mode, 'failed');

  await step(20, { playing: false, seek: 2 });
  assert.equal(page.view(1).mode, 'picture', 'entering seek 2 replays the healthy held seek-2 panel');
  assert.equal((page.view(1).fields?.['model:3'] as { structure?: string }).structure, 'healthy2');
  assert.equal(totalReleased() + lane.retainedBytes(), totalCharged);
});

test('F8: refusal for a previous binding arriving after switching to a healthy subscription never halts the current binding', async () => {
  let finishFirstCrop!: () => void;
  let cropCall = 0;
  const view: LaneView = {
    paint: (input): Painted => ({
      surfaces: [{ source: { width: 1, height: 1, content: `v:${input.subs[0]!.id}:${input.pictures[0]!.h}` } as never, content: `v:${input.subs[0]!.id}:${input.pictures[0]!.h}` }],
      fields: {},
      motion: false,
    }),
  };
  const { page, lane, mount, step } = rig({ view }, {
    crop: sources => {
      cropCall++;
      const make = () => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: 1, close() { this.closed = true; } }));
      if (cropCall === 1) return new Promise<Fake[]>(resolve => { finishFirstCrop = () => resolve(make()); });
      return Promise.resolve(make());
    },
  });
  mount(1, [{ id: 3, channel: 'followed' }], 'view', 1);
  lane.bindRoute(3, ROUTE, NATIVE, 0);
  lane.receive('follow', message(10, [{ id: 3, answer: rows([1n], [10n]) }], false, NATIVE, 0));
  page.publish(10, { playing: false, seek: 0 });
  await flush();
  page.name(1, 'failed', 'sub 3 failed');
  finishFirstCrop();
  await flush();

  lane.follow(5, 'followed');
  lane.bindRoute(5, ROUTE, NATIVE, 0);
  lane.mount(1, [5], 'view');
  page.rebind(1, { subscription: true });
  lane.bind(1, { generation: page.widgets.get(1)!.binding, surfaces: [{ width: 1, height: 1, dpr: 1 }], offset: 0, pin: null });
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n], [50n]) }], false, NATIVE, 0));
  await step(10, { playing: false });
  await step(10, { playing: false });
  assert.equal(page.view(1).mode, 'picture', 'stale refusal from binding 1 must never halt binding 2');
});

test('F9: releasing and reopening the last subscription retires old page routes so frames send only active reset pairs', async () => {
  const { lane, presented, mount, step } = rig();
  for (let r = 1; r <= 128; r++) {
    const sub = r * 2 + 1;
    lane.follow(sub, 'followed');
    lane.bindRoute(sub, r, NATIVE, 0);
    lane.unfollow(sub);
  }
  mount(1, [{ id: 501, channel: 'followed' }]);
  lane.bindRoute(501, 129, NATIVE, 0);
  lane.receive('follow', message(10, [{ id: 501, answer: rows([1n], [10n]) }], false, NATIVE, 0));
  presented.length = 0;
  await step(10, { playing: false });
  assert.ok(presented.length >= 1);
  assert.deepEqual(presented.at(-1)!.resets, [[129, 0]], 'retired page routes must not accumulate in presentation reset pairs');
});

test('post-serve deferred frame whose seek was superseded by stream before run never paints or prunes the new generation, still applies acks and emits served', async () => {
  const scheduled: (() => void)[] = [];
  const presented: Presented<Fake>[] = [];
  const served: { seek: number; target: number; exact: boolean; changed: number; ms: number }[] = [];
  const released: number[] = [];
  let cropCall = 0;
  let resolveSecondCrop!: () => void;
  const view: LaneView = {
    paint: (input): Painted => {
      const pic = input.pictures[0]!;
      const val = pic.parts[0]!.value;
      const body = val.kind === 'answer' ? val.body : undefined;
      return {
        surfaces: [{ source: { width: 8, height: 8, content: `h:${pic.h}` } as never, content: `h:${pic.h}` }],
        fields: body ? { 'answer:5': { value: body, content: `a:${pic.h}` } } : {},
        motion: false,
      };
    },
  };
  const lane = new Lane<Fake>({
    lane: 0,
    lanes: 1,
    views: () => view,
    crop: sources => {
      cropCall++;
      const make = () => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: source.width, close() { this.closed = true; } }));
      if (cropCall === 2) return new Promise<Fake[]>(resolve => { resolveSecondCrop = () => resolve(make()); });
      return Promise.resolve(make());
    },
    schedule: run => { scheduled.push(run); },
    clock: () => 0,
    present: presentation => { presented.push(presentation); },
    release: bytes => { released.push(bytes); },
    fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); },
    later: () => {},
    served: frame => { served.push(frame); },
  });
  lane.follow(5, 'followed');
  lane.mount(1, [5], 'view');
  lane.bind(1, { generation: 1, surfaces: [{ width: 8, height: 8, dpr: 1 }], offset: 0, pin: null });
  lane.bindRoute(5, ROUTE, NATIVE, 0);

  lane.receive('follow', message(990, [{ id: 5, hour: 990, answer: rows([1n], [990n]) }], false, NATIVE, 0));
  lane.frame({ type: 'frame', serial: 1, now: 990, target: 990, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
  await flush();
  assert.equal(presented.length, 1);
  assert.equal(scheduled.length, 0, 'idle lane uses microtask, not host.schedule');
  const firstEntryId = presented[0]!.entries[0]!.id;

  lane.receive('follow', message(995, [{ id: 5, hour: 995, answer: rows([1n], [995n]) }], false, NATIVE, 0));
  lane.frame({ type: 'frame', serial: 2, now: 995, target: 995, exact: false, seek: 0, speed: 1440, installed: [], refused: [], inspect: [] });
  await flush();
  assert.equal(cropCall, 2, 'second serve suspended in crop');

  lane.frame({
    type: 'frame', serial: 3, now: 1000, target: 1024, exact: false, seek: 0, speed: 1440,
    installed: [{ widget: 1, binding: 1, id: firstEntryId }], refused: [], inspect: [],
  });
  resolveSecondCrop();
  await flush();
  assert.equal(scheduled.length, 1, 'queued seek-0 playing frame is deferred via post-serve host.schedule');

  lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE, 1));
  lane.receive('follow', message(11, [{ id: 5, hour: 11, answer: rows([1n], [11n]) }], false, NATIVE, 1));
  lane.receive('follow', message(12, [{ id: 5, hour: 12, answer: rows([1n], [12n]) }], false, NATIVE, 1));
  lane.stream(ROUTE, 1, 10);

  const presentedBefore = presented.length;
  const servedBefore = served.length;
  const releasedBefore = released.reduce((sum, b) => sum + b, 0);
  scheduled.shift()!();
  await flush();

  assert.equal(presented.length, presentedBefore, 'superseded seek-0 request must not paint under seek 1');
  assert.equal(served.length, servedBefore + 1, 'superseded seek-0 request still emits served');
  assert.deepEqual([served.at(-1)!.seek, served.at(-1)!.changed], [0, 0]);
  assert.equal(lane.facts.select(5, 10)?.h, 10, 'superseded seek-0 request must not prune hour 10 from seek 1');

  lane.frame({ type: 'frame', serial: 4, now: 10, target: 10, exact: true, seek: 1, speed: 0, installed: [], refused: [], inspect: [] });
  await flush();
  while (scheduled.length) { scheduled.shift()!(); await flush(); }
  assert.equal(presented.length, presentedBefore + 1, 'following exact seek-1 request for hour 10 paints hour 10');
  const seek1Entry = presented.at(-1)!.entries[0]!;
  assert.deepEqual([seek1Entry.gen.seek, seek1Entry.h], [1, 10]);
  const answer = parseAnswer(seek1Entry.fields['answer:5'] as Uint8Array);
  assert.ok(answer.kind === 'rows' && cellValue(answer, 0) === 10n);
  lane.frame({
    type: 'frame', serial: 5, now: 10, target: 10, exact: true, seek: 1, speed: 0,
    installed: [{ widget: 1, binding: 1, id: seek1Entry.id }], refused: [], inspect: [],
  });
  await flush();
  while (scheduled.length) { scheduled.shift()!(); await flush(); }
  assert.ok(released.reduce((sum, b) => sum + b, 0) > releasedBefore, 'acknowledged seek-0 fact is released once replaced');
});

test('startLaneWorker with macrotaskSchedule keeps Page-installed picture age within two paint intervals at every tick, while microtask fallback falls linearly behind', async () => {
  assert.equal(macrotaskSchedule(null), undefined, 'macrotaskSchedule returns undefined when MessageChannel is unavailable');

  const closed: number[] = [];
  await assert.rejects(
    cropAll([1, 2, 3], async id => {
      if (id === 2) throw new Error('crop boom');
      return { close() { closed.push(id); } };
    }),
    /crop boom/,
  );
  assert.deepEqual(closed, [1, 3], 'cropAll closes all fulfilled sibling bitmaps when any item rejects');

  const simulate = async (withMessageChannel: boolean) => {
    const frameMs = 16;
    const paintMs = 7;
    const stretchFactor = 5;
    const totalServeMs = paintMs * (1 + stretchFactor);
    const stepHours = 24;
    const ticks = 80;
    let clockMs = 0;
    let paints = 0;
    let pushed = -stepHours;
    let seq = 0;
    const mailbox: { at: number; seq: number; run: () => void }[] = [];
    const pendingOffers: { at: number; msg: Presented<ImageBitmap> }[] = [];
    const tickAges: number[] = [];
    const port = { onmessage: null as ((event: MessageEvent<FromPull>) => void) | null, postMessage() {} } as unknown as MessagePort;
    const enqueue = (at: number, run: () => void) => { mailbox.push({ at, seq: seq++, run }); };

    const pushFactsThrough = (at: number, limit: number) => {
      while (pushed + stepHours <= limit) {
        pushed += stepHours;
        const h = pushed;
        const pkt = message(h + stepHours - 1, [{ id: 5, hour: h, answer: rows([1n], [BigInt(h)]) }], false, NATIVE);
        enqueue(at, () => { port.onmessage?.({ data: { kind: 'follow', bytes: pkt } } as MessageEvent<FromPull>); });
      }
    };

    let publishAt = 0;
    let dispatch!: (message: ToLane) => void;
    const page = new Page<ImageBitmap>({
      lanes: 1,
      post: (_lane, m: PageMessage) => {
        const at = publishAt;
        if (m.type === 'frame') enqueue(at, () => { dispatch(m); });
        else if (m.type === 'stream') enqueue(at, () => { dispatch({ type: 'stream', route: m.route, seek: m.seek, target: m.target }); });
      },
    });
    page.mount(1, { route: ROUTE, lane: 0, surfaces: [{ install() {}, clear() {} }] });

    class FakeMessageChannel implements WakeChannel {
      readonly port1: WakeChannel['port1'] = { onmessage: null };
      readonly port2: WakeChannel['port2'] = {
        postMessage: (data: unknown) => {
          enqueue(clockMs, () => { this.port1.onmessage?.({ data } as MessageEvent); });
        },
      };
    }

    const view: LaneView = {
      paint: (input): Painted => {
        paints++;
        clockMs += paintMs;
        const h = input.pictures[0]!.h;
        return {
          surfaces: [{ source: { width: 10, height: 10 } as never, content: `h:${h}` }],
          fields: { h: { value: h, content: `h:${h}` } },
          motion: false,
        };
      },
    };

    const originalChannel = globalThis.MessageChannel;
    try {
      (globalThis as { MessageChannel?: unknown }).MessageChannel = withMessageChannel ? FakeMessageChannel : undefined;
      dispatch = startLaneWorker((msg: FromLane) => {
        if (msg.type === 'presentation') pendingOffers.push({ at: clockMs, msg });
      }, {
        views: () => view,
        crop: async sources => sources.map(source => ({ width: source.width, height: source.height, close() {} }) as unknown as ImageBitmap),
        later: () => {},
        clock: () => clockMs,
        stretch: ms => { clockMs += ms * stretchFactor; },
      });
    } finally {
      (globalThis as { MessageChannel?: unknown }).MessageChannel = originalChannel;
    }

    const drainMicrotasks = async () => {
      for (let i = 0; i < 6; i++) await Promise.resolve();
    };
    const popDue = (limitMs: number) => {
      let best = -1;
      for (let i = 0; i < mailbox.length; i++) {
        const item = mailbox[i]!;
        if (item.at > limitMs) continue;
        if (best < 0 || item.at < mailbox[best]!.at || (item.at === mailbox[best]!.at && item.seq < mailbox[best]!.seq)) best = i;
      }
      return best < 0 ? null : mailbox.splice(best, 1)[0]!;
    };

    dispatch({ type: 'init', lane: 0, lanes: 1, port, rate: 1 });
    dispatch({ type: 'follow', sub: 5, channel: 'followed' });
    dispatch({ type: 'mount', widget: 1, subs: [5], view: 'slow' });
    dispatch({ type: 'bind', widget: 1, binding: { generation: 1, surfaces: [{ width: 10, height: 10, dpr: 1 }], offset: 0, pin: null } });
    dispatch({ type: 'route', sub: 5, route: ROUTE, native: NATIVE, seek: 0, offset: 0 });
    await drainMicrotasks();

    publishAt = 0;
    pushFactsThrough(0, 4 * stepHours);
    page.publish(0, { playing: false, step: stepHours, speed: 0 });
    while (mailbox.length) {
      mailbox.shift()!.run();
      await drainMicrotasks();
    }
    while (pendingOffers.length) page.offer(pendingOffers.shift()!.msg);
    const warmupTicks = 20;
    for (let sampleTick = 1; sampleTick <= warmupTicks + ticks; sampleTick++) {
      const targetMs = sampleTick * frameMs;
      const nowHour = sampleTick * stepHours;
      for (let i = 0; i < pendingOffers.length; ) {
        if (pendingOffers[i]!.at <= targetMs) {
          page.offer(pendingOffers.splice(i, 1)[0]!.msg);
        } else i++;
      }
      publishAt = targetMs;
      pushFactsThrough(targetMs, nowHour + 4 * stepHours);
      page.publish(nowHour, { playing: true, step: stepHours, speed: 1440 });
      if (sampleTick === warmupTicks + 1) paints = 0;
      if (sampleTick > warmupTicks) {
        const installedHour = page.view(1).h;
        tickAges.push(typeof installedHour === 'number' ? nowHour - installedHour : Infinity);
      }
      while (clockMs <= targetMs) {
        const task = popDue(targetMs);
        if (!task) break;
        if (clockMs < task.at) clockMs = task.at;
        task.run();
        await drainMicrotasks();
      }
    }

    const finalHour = (warmupTicks + ticks) * stepHours;
    const paintBudgetHours = Math.ceil((totalServeMs / frameMs) * stepHours);
    return {
      paints,
      maxAge: Math.max(...tickAges),
      endAge: tickAges.at(-1)!,
      installedHour: page.view(1).h ?? 0,
      finalHour,
      paintBudgetHours,
    };
  };

  const macro = await simulate(true);
  assert.ok(macro.maxAge <= macro.paintBudgetHours * 2,
    `worker macrotask schedule must keep Page-installed age bounded (${macro.paintBudgetHours * 2}h) at every tick, got max=${macro.maxAge}h end=${macro.endAge}h paints=${macro.paints} installed=${macro.installedHour}`);
  assert.ok(macro.installedHour >= macro.finalHour - macro.paintBudgetHours * 2,
    `worker macrotask schedule must progress Page-installed picture through the final hour (${macro.finalHour}h), got installed=${macro.installedHour}h`);
  assert.ok(macro.paints < 40 && macro.paints >= 15, `worker macrotask schedule must drop intermediate frames without stalling, painted ${macro.paints} of 80 ticks`);

  const micro = await simulate(false);
  assert.ok(micro.endAge > 1000,
    `regression proof: without MessageChannel microtask schedule must fall linearly behind (> 1000h), got endAge=${micro.endAge}h`);
});

test('idle lane runs frame before subsequent messages: pointer after ack, refusal then install, and exact then speculative', async () => {
  const drainMicrotasks = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
  const makeIdleHarness = (view: LaneView) => {
    const mailbox: (() => void)[] = [];
    const hits: (string | null)[] = [];
    const presented: Presented<Fake>[] = [];
    const lane = new Lane<Fake>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      schedule: run => { mailbox.push(run); },
      present: presentation => { presented.push(presentation); },
      release: () => {},
      fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); },
      later: () => {},
      hit: (_widget, key) => { hits.push(key); },
    });
    const runMailbox = async () => {
      while (mailbox.length) {
        mailbox.shift()!();
        await drainMicrotasks();
      }
    };
    return { lane, mailbox, hits, presented, runMailbox };
  };

  {
    const view: LaneView = {
      paint: (input): Painted => ({
        surfaces: [],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
        hit: () => `row:${input.pictures[0]!.h}`,
      }),
    };
    const { lane, mailbox, hits, presented, runMailbox } = makeIdleHarness(view);
    lane.follow(3, 'panel');
    lane.mount(1, [3], 'view');
    lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    lane.bindRoute(3, ROUTE, NATIVE);
    lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }]));
    mailbox.push(() => lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] }));
    await runMailbox();
    const id10 = presented.at(-1)!.entries[0]!.id;
    mailbox.push(() => lane.frame({ type: 'frame', serial: 2, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [{ widget: 1, binding: 1, id: id10 }], refused: [], inspect: [] }));
    await runMailbox();

    lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: {} }]));
    mailbox.push(() => lane.frame({ type: 'frame', serial: 3, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] }));
    await runMailbox();
    const id20 = presented.at(-1)!.entries[0]!.id;

    mailbox.push(() => lane.frame({ type: 'frame', serial: 4, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [{ widget: 1, binding: 1, id: id20 }], refused: [], inspect: [] }));
    mailbox.push(() => lane.pointer(1, 0, [1, 1]));
    await runMailbox();
    assert.deepEqual(hits, ['row:20'], 'pointer after an acknowledging frame on an idle lane hits the newly acknowledged picture');
  }

  {
    const view: LaneView = {
      paint: (input): Painted => ({
        surfaces: [],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
        hit: () => `row:${input.pictures[0]!.h}`,
      }),
    };
    const { lane, mailbox, hits, presented, runMailbox } = makeIdleHarness(view);
    lane.follow(3, 'panel');
    lane.mount(1, [3], 'view');
    lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    lane.bindRoute(3, ROUTE, NATIVE);
    lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }]));
    mailbox.push(() => lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] }));
    await runMailbox();
    const id10 = presented.at(-1)!.entries[0]!.id;

    lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: {} }]));
    mailbox.push(() => lane.frame({ type: 'frame', serial: 2, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] }));
    await runMailbox();
    const id20 = presented.at(-1)!.entries[0]!.id;

    mailbox.push(() => lane.frame({ type: 'frame', serial: 3, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [], refused: [{ widget: 1, id: id10, reason: 'kept content' }], inspect: [] }));
    mailbox.push(() => lane.frame({ type: 'frame', serial: 4, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [{ widget: 1, binding: 1, id: id20 }], refused: [], inspect: [] }));
    mailbox.push(() => lane.pointer(1, 0, [1, 1]));
    await runMailbox();
    assert.deepEqual(hits, ['row:20'], 'refusal frame then install frame on an idle lane run as separate serves so hit-test returns row:20');
  }

  {
    const view: LaneView = {
      paint: (input): Painted => ({
        surfaces: [],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
      }),
    };
    const { lane, mailbox, presented, runMailbox } = makeIdleHarness(view);
    lane.follow(5, 'followed');
    lane.mount(1, [5], 'view');
    lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    lane.bindRoute(5, ROUTE, NATIVE);
    lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE));
    lane.receive('follow', message(12, [{ id: 5, hour: 12, answer: rows([1n], [12n]) }], false, NATIVE));

    mailbox.push(() => lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 1, installed: [], refused: [], inspect: [] }));
    mailbox.push(() => lane.frame({ type: 'frame', serial: 2, now: 10, target: 12, exact: false, seek: 0, speed: 1, installed: [], refused: [], inspect: [] }));
    await runMailbox();
    assert.deepEqual(presented.map(p => p.entries[0]?.h), [10, 12], 'exact frame then speculative frame on an idle lane both serve so hour 10 is offered before hour 12');
  }
});

test('held crop superseded by a rewind stream never prunes the new seek generation and closes its bitmaps', async () => {
  let resolveCrop!: (bitmaps: Fake[]) => void;
  let cropCall = 0;
  const createdBitmaps: Fake[] = [];
  const presented: Presented<Fake>[] = [];
  const served: { seek: number; target: number; exact: boolean; changed: number; cancelled?: boolean }[] = [];
  const mailbox: (() => void)[] = [];
  const view: LaneView = {
    paint: (input): Painted => ({
      surfaces: [{ source: { width: 10, height: 10, content: `h:${input.pictures[0]!.h}` } as never, content: `h:${input.pictures[0]!.h}` }],
      fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
      motion: false,
    }),
  };
  const lane = new Lane<Fake>({
    lane: 0,
    lanes: 1,
    views: () => view,
    crop: sources => {
      cropCall++;
      const make = () => sources.map((source): Fake => {
        const bm: Fake = { content: (source as unknown as { content: string }).content, closed: false, width: source.width, close() { this.closed = true; } };
        createdBitmaps.push(bm);
        return bm;
      });
      if (cropCall === 1) return new Promise<Fake[]>(resolve => { resolveCrop = () => resolve(make()); });
      return Promise.resolve(make());
    },
    schedule: run => { mailbox.push(run); },
    present: presentation => { presented.push(presentation); },
    release: () => {},
    fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); },
    later: () => {},
    served: frame => { served.push(frame); },
  });
  lane.follow(5, 'followed');
  lane.mount(1, [5], 'view');
  lane.bind(1, { generation: 1, surfaces: [{ width: 10, height: 10, dpr: 1 }], offset: 0, pin: null });
  lane.bindRoute(5, ROUTE, NATIVE, 0);

  lane.receive('follow', message(1000, [{ id: 5, hour: 1000, answer: rows([1n], [1000n]) }], false, NATIVE, 0));
  lane.frame({ type: 'frame', serial: 1, now: 1000, target: 1000, exact: false, seek: 0, speed: 1440, installed: [], refused: [], inspect: [] });
  await flush();
  while (mailbox.length) { mailbox.shift()!(); await flush(); }
  assert.equal(cropCall, 1, 'seek-0 frame started painting and suspended in crop');

  lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE, 1));
  lane.receive('follow', message(11, [{ id: 5, hour: 11, answer: rows([1n], [11n]) }], false, NATIVE, 1));
  lane.receive('follow', message(12, [{ id: 5, hour: 12, answer: rows([1n], [12n]) }], false, NATIVE, 1));
  lane.stream(ROUTE, 1, 10);
  lane.frame({ type: 'frame', serial: 2, now: 10, target: 10, exact: true, seek: 1, speed: 0, installed: [], refused: [], inspect: [] });

  resolveCrop([]);
  await flush();
  while (mailbox.length) { mailbox.shift()!(); await flush(); }

  assert.equal(createdBitmaps[0]!.closed, true, 'dropped superseded crop bitmap must be closed with no leak');
  assert.equal(presented.filter(p => p.entries.some(e => e.gen.seek === 0)).length, 0, 'superseded seek-0 presentation must not be emitted');
  assert.equal(lane.facts.select(5, 10)?.h, 10, 'superseded seek-0 crop resolution must not prune hour 10 from seek 1');
  const seek1 = presented.find(p => p.entries.some(e => e.gen.seek === 1));
  assert.ok(seek1, 'exact seek-1 request for hour 10 must produce an entry');
  assert.equal(seek1!.entries[0]!.h, 10);
  assert.equal(seek1!.entries[0]!.surfaces[0] !== 'same', true, 'seek-1 entry must carry a real bitmap, never SAME against the dropped seek-0 crop');
  assert.equal(served.some(s => s.seek === 0 && s.changed === 0 && s.cancelled === true), true, 'cancelled post-crop paint must report cancelled: true with changed: 0');
});

test('hand-over after a slow serve preserves pointer-after-ack, refusal-then-install, exact-then-speculative, and arms motion at serve completion', async () => {
  const drainMicrotasks = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
  const makeSlowHarness = (view: LaneView, paintCostMs: () => number) => {
    let clockMs = 0;
    const wakes: (() => void)[] = [];
    const laters: (() => void)[] = [];
    const hits: (string | null)[] = [];
    const presented: Presented<Fake>[] = [];
    const served: { seek: number; target: number; exact: boolean; changed: number }[] = [];
    const wrappedView: LaneView = {
      paint: (input, state, content) => {
        clockMs += paintCostMs();
        return view.paint(input, state, content);
      },
    };
    const lane = new Lane<Fake>({
      lane: 0,
      lanes: 1,
      views: () => wrappedView,
      crop: async () => [],
      clock: () => clockMs,
      schedule: run => { wakes.push(run); },
      present: presentation => { presented.push(presentation); },
      release: () => {},
      fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); },
      later: run => { laters.push(run); },
      hit: (_widget, key) => { hits.push(key); },
      served: frame => { served.push(frame); },
    });
    const drainAll = async () => {
      await drainMicrotasks();
      while (wakes.length) {
        wakes.shift()!();
        await drainMicrotasks();
      }
    };
    return { lane, wakes, laters, hits, presented, served, drainMicrotasks, drainAll };
  };

  {
    let cost = 1;
    const view: LaneView = {
      paint: (input): Painted => ({
        surfaces: [],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
        hit: () => `row:${input.pictures[0]!.h}`,
      }),
    };
    const { lane, wakes, hits, presented, drainMicrotasks, drainAll } = makeSlowHarness(view, () => cost);
    lane.follow(3, 'panel');
    lane.mount(1, [3], 'view');
    lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    lane.bindRoute(3, ROUTE, NATIVE);
    lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }]));
    lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await drainAll();
    const id10 = presented.at(-1)!.entries[0]!.id;
    lane.frame({ type: 'frame', serial: 2, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [{ widget: 1, binding: 1, id: id10 }], refused: [], inspect: [] });
    await drainAll();

    cost = 30;
    lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: {} }]));
    lane.frame({ type: 'frame', serial: 3, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    assert.equal(wakes.length, 1, '30 ms serve opens hand-over wake');
    const id20 = presented.at(-1)!.entries[0]!.id;

    lane.frame({ type: 'frame', serial: 4, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [{ widget: 1, binding: 1, id: id20 }], refused: [], inspect: [] });
    lane.pointer(1, 0, [1, 1]);
    assert.deepEqual(hits, ['row:20'], 'pointer after ack during hand-over before wake hits row:20');
  }

  {
    let cost = 1;
    const view: LaneView = {
      paint: (input): Painted => ({
        surfaces: [],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
        hit: () => `row:${input.pictures[0]!.h}`,
      }),
    };
    const { lane, wakes, hits, presented, drainMicrotasks, drainAll } = makeSlowHarness(view, () => cost);
    lane.follow(3, 'panel');
    lane.mount(1, [3], 'view');
    lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    lane.bindRoute(3, ROUTE, NATIVE);
    lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }]));
    lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await drainAll();
    const id10 = presented.at(-1)!.entries[0]!.id;

    cost = 30;
    lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: {} }]));
    lane.frame({ type: 'frame', serial: 2, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    assert.equal(wakes.length, 1, '30 ms serve opens hand-over wake');
    const id20 = presented.at(-1)!.entries[0]!.id;

    cost = 1;
    lane.frame({ type: 'frame', serial: 3, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [], refused: [{ widget: 1, id: id10, reason: 'kept content' }], inspect: [] });
    lane.frame({ type: 'frame', serial: 4, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [{ widget: 1, binding: 1, id: id20 }], refused: [], inspect: [] });
    await drainAll();
    lane.pointer(1, 0, [1, 1]);
    assert.deepEqual(hits, ['row:20'], 'kept content refusal then install during hand-over leaves row:20 installed');
  }

  {
    let cost = 30;
    const view: LaneView = {
      paint: (input): Painted => ({
        surfaces: [],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
      }),
    };
    const { lane, wakes, presented, served, drainMicrotasks, drainAll } = makeSlowHarness(view, () => cost);
    lane.follow(5, 'followed');
    lane.mount(1, [5], 'view');
    lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    lane.bindRoute(5, ROUTE, NATIVE);
    lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE));
    lane.receive('follow', message(30, [{ id: 5, hour: 30, answer: rows([1n], [30n]) }], false, NATIVE));
    lane.receive('follow', message(40, [{ id: 5, hour: 40, answer: rows([1n], [40n]) }], false, NATIVE));

    lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: false, seek: 0, speed: 1440, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    assert.equal(wakes.length, 1, '30 ms serve opens hand-over wake');

    cost = 1;
    const presentedBefore = presented.length;
    const servedBefore = served.length;
    lane.frame({ type: 'frame', serial: 2, now: 30, target: 30, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    assert.deepEqual(presented.slice(presentedBefore).map(p => p.entries[0]?.h), [30], 'exact 30 arriving during hand-over serves immediately without waiting for the wake');
    const lateWake = wakes.shift()!;
    lateWake();
    await drainMicrotasks();
    assert.equal(served.length, servedBefore + 1, 'late hand-over wake after exact frame is a no-op');

    lane.frame({ type: 'frame', serial: 3, now: 30, target: 40, exact: false, seek: 0, speed: 1440, installed: [], refused: [], inspect: [] });
    await drainAll();
    assert.deepEqual(presented.slice(presentedBefore).map(p => p.entries[0]?.h), [30, 40], 'exact 30 then speculative 40 during hand-over both present');
    assert.deepEqual(served.slice(servedBefore).map(s => [s.target, s.exact]), [[30, true], [40, false]], 'both exact 30 and speculative 40 emit served');
  }

  {
    let resolveCrop!: () => void;
    let cropCall = 0;
    const wakes: (() => void)[] = [];
    const presented: Presented<Fake>[] = [];
    const view: LaneView = {
      paint: (input): Painted => ({
        surfaces: [{ source: { width: 4, height: 4, content: `h:${input.pictures[0]!.h}` } as never, content: `h:${input.pictures[0]!.h}` }],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
      }),
    };
    const lane = new Lane<Fake>({
      lane: 0, lanes: 1, views: () => view, clock: () => 0,
      crop: sources => {
        cropCall++;
        const make = () => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: source.width, close() { this.closed = true; } }));
        if (cropCall === 1) return new Promise<Fake[]>(resolve => { resolveCrop = () => resolve(make()); });
        return Promise.resolve(make());
      },
      schedule: run => { wakes.push(run); },
      present: presentation => { presented.push(presentation); },
      release: () => {}, fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); }, later: () => {},
    });
    lane.follow(5, 'followed');
    lane.mount(1, [5], 'view');
    lane.bind(1, { generation: 1, surfaces: [{ width: 4, height: 4, dpr: 1 }], offset: 0, pin: null });
    lane.bindRoute(5, ROUTE, NATIVE);
    lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE));
    lane.receive('follow', message(20, [{ id: 5, hour: 20, answer: rows([1n], [20n]) }], false, NATIVE));

    lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: false, seek: 0, speed: 1440, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    assert.equal(cropCall, 1);
    lane.frame({ type: 'frame', serial: 2, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    resolveCrop();
    await drainMicrotasks();
    assert.equal(wakes.length, 0, 'exact request pending at serve completion starts from a microtask without a hand-over wake');
    assert.deepEqual(presented.map(p => p.entries[0]?.h), [10, 20]);
  }

  {
    const view: LaneView = {
      paint: (input): Painted => ({
        surfaces: [],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: true,
      }),
    };
    const { lane, wakes, laters, drainMicrotasks } = makeSlowHarness(view, () => 30);
    lane.follow(5, 'followed');
    lane.mount(1, [5], 'view');
    lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    lane.bindRoute(5, ROUTE, NATIVE);
    lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE));

    lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    assert.equal(wakes.length, 1, '30 ms motion serve opens hand-over wake');
    assert.equal(laters.length, 1, 'motion callback is armed at serve completion before the hand-over wake runs');
  }
});

test('an older kept-content refusal in the same batch preserves the newer installed picture', async () => {
  const { lane, mount, presented, hits } = rig({ probe: {
    paint: ({ hour }) => ({ surfaces: [], fields: {}, motion: false, hit: () => String(hour) }),
  } });
  mount(1, [{ id: 3, channel: 'panel' }], 'probe');
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([10, 20].map(hour => ({ id: 3, hour, kind: 'data', stepped: true, data: { hour } }))));
  const frame: FrameRequest = { type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] };
  lane.frame(frame);
  await flush();
  const older = presented.at(-1)!.entries[0]!;
  const next = { ...frame, serial: 2, now: 20, target: 20 };
  lane.frame(next);
  await flush();
  const newer = presented.at(-1)!.entries[0]!;
  assert.ok(newer.id > older.id);

  const before = presented.length;
  lane.frame({ ...next, serial: 3, installed: [{ widget: 1, binding: 1, id: newer.id }], refused: [{ widget: 1, id: older.id, reason: 'kept content' }] });
  lane.frame({ ...next, serial: 4 });
  await flush();
  lane.pointer(1, 0, [0, 0]);
  assert.deepEqual(hits, ['20']);
  assert.equal(presented.length, before);
});

test('a queued exact frame installs the current hour before its speculative successor is due', async () => {
  const { lane, mount, page } = rig({ probe: {
    paint: () => ({ surfaces: [], fields: { v: { value: 100, content: 'shared' } }, motion: false }),
  } });
  mount(1, [{ id: 3, channel: 'panel' }], 'probe');
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([10, 11, 12].map(hour => ({ id: 3, hour, kind: 'data', stepped: true, data: { hour } }))));
  page.publish(10, { playing: true, speed: 24, step: 1 });
  page.publish(11);
  await flush();
  page.frame();
  assert.deepEqual([page.view(1).h, page.view(1).fields?.v], [11, 100]);
  assert.equal(page.ahead[0], 1, 'both pictures answered the current tick');
  page.publish(12);
  assert.deepEqual([page.view(1).h, page.view(1).fields?.v], [12, 100]);
});

test('mid-serve ack FIFO flushes at serve completion before wake, and resuming during crop installs hour 20 on Page', async () => {
  const drainMicrotasks = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };

  {
    let resolveHeldCrop!: () => void;
    let cropCall = 0;
    const wakes: (() => void)[] = [];
    const hits: (string | null)[] = [];
    const presented: Presented<Fake>[] = [];
    const view: LaneView = {
      paint: (input): Painted => ({
        surfaces: [{ source: { width: 4, height: 4, content: `h:${input.pictures[0]!.h}` } as never, content: `h:${input.pictures[0]!.h}` }],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
        hit: () => `row:${input.pictures[0]!.h}`,
      }),
    };
    const lane = new Lane<Fake>({
      lane: 0, lanes: 1, views: () => view, clock: () => 0,
      crop: sources => {
        cropCall++;
        const make = () => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: source.width, close() { this.closed = true; } }));
        if (cropCall === 4) return new Promise<Fake[]>(resolve => { resolveHeldCrop = () => resolve(make()); });
        return Promise.resolve(make());
      },
      schedule: run => { wakes.push(run); },
      present: presentation => { presented.push(presentation); },
      release: () => {}, fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); }, later: () => {},
      hit: (_widget, key) => { hits.push(key); },
    });
    lane.follow(3, 'panel');
    lane.mount(1, [3], 'view');
    lane.bind(1, { generation: 1, surfaces: [{ width: 4, height: 4, dpr: 1 }], offset: 0, pin: null });
    lane.bindRoute(3, ROUTE, NATIVE);

    lane.receive('panels', panels([{ id: 3, hour: 5, kind: 'data', stepped: true, data: {} }]));
    lane.frame({ type: 'frame', serial: 1, now: 5, target: 5, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    const idBase = presented.at(-1)!.entries[0]!.id;
    lane.frame({ type: 'frame', serial: 2, now: 5, target: 5, exact: true, seek: 0, speed: 0, installed: [{ widget: 1, binding: 1, id: idBase }], refused: [], inspect: [] });
    await drainMicrotasks();

    lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }]));
    lane.frame({ type: 'frame', serial: 3, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    const id10 = presented.at(-1)!.entries[0]!.id;

    lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: {} }]));
    lane.frame({ type: 'frame', serial: 4, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    const id20 = presented.at(-1)!.entries[0]!.id;

    lane.receive('panels', panels([{ id: 3, hour: 30, kind: 'data', stepped: true, data: {} }]));
    lane.frame({ type: 'frame', serial: 5, now: 30, target: 30, exact: false, seek: 0, speed: 1440, installed: [], refused: [], inspect: [] });
    await drainMicrotasks();
    assert.equal(cropCall, 4, 'fourth serve is suspended in crop while both id10 and id20 remain outstanding');

    lane.frame({ type: 'frame', serial: 6, now: 20, target: 20, exact: false, seek: 0, speed: 1440, installed: [], refused: [{ widget: 1, id: id20, reason: 'kept content' }], inspect: [] });
    lane.frame({ type: 'frame', serial: 7, now: 20, target: 20, exact: false, seek: 0, speed: 1440, installed: [{ widget: 1, binding: 1, id: id10 }], refused: [], inspect: [] });

    lane.pointer(1, 0, [1, 1]);
    assert.deepEqual(hits, ['row:5'], 'neither queued ack batch applies while crop is still held');

    resolveHeldCrop();
    await drainMicrotasks();
    assert.equal(wakes.length, 1, 'speculative hand-over wake is held');
    lane.pointer(1, 0, [1, 1]);
    assert.deepEqual(hits, ['row:5', 'row:10'], 'the second batch installs hour 10 after the first batch refused hour 20');
    assert.equal(lane.stats.refused['kept content'], 1);
  }

  {
    let resolveHeldCrop!: () => void;
    let cropCall = 0;
    const view: LaneView = {
      paint: (input): Painted => {
        const val = input.pictures[0]!.parts[0]!.value;
        const num = val.kind === 'answer' && val.answer.kind === 'rows' ? cellValue(val.answer, 0) : 0n;
        const key = `content:${num}`;
        return {
          surfaces: [{ source: { width: 8, height: 8, content: key } as never, content: key }],
          fields: { v: { value: num, content: key } },
          motion: false,
        };
      },
    };
    const { page, lane, mount } = rig({ view }, {
      crop: sources => {
        cropCall++;
        const make = () => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: source.width, close() { this.closed = true; } }));
        if (cropCall === 1) return new Promise<Fake[]>(resolve => { resolveHeldCrop = () => resolve(make()); });
        return Promise.resolve(make());
      },
    });
    mount(1, [{ id: 5, channel: 'followed' }], 'view', 1);
    lane.bindRoute(5, ROUTE, NATIVE, 0);

    lane.receive('follow', message(5, [{ id: 5, hour: 5, answer: rows([1n], [50n]) }], false, NATIVE, 0));
    page.publish(5, { playing: false, seek: 0 });
    await flush();
    assert.equal(cropCall, 1, 'hour-5 crop is held');

    lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [100n]) }], false, NATIVE, 0));
    lane.receive('follow', message(21, [{ id: 5, hour: 20, answer: rows([1n], [100n]) }], false, NATIVE, 0));
    page.publish(10, { playing: true, step: 1, speed: 24, seek: 0 });
    page.publish(20, { playing: true, step: 1, speed: 24, seek: 0 });

    resolveHeldCrop();
    await flush();
    await flush();
    page.frame();
    assert.equal(page.view(1).mode, 'picture', 'Page installs hour 20 instead of rejecting SAME against a superseded exact hour 10');
    assert.equal(page.view(1).h, 20);
  }
});

test('same-content rewind after held crop ignores queued stream refusal and Page installs hour 10 on the first attempt', async () => {
  let resolveHeldCrop!: () => void;
  let cropCall = 0;
  let delayNextPresentation = false;
  const delayedPresentations: Presented<Fake>[] = [];
  const view: LaneView = {
    paint: (input): Painted => {
      const val = input.pictures[0]!.parts[0]!.value;
      const num = val.kind === 'answer' && val.answer.kind === 'rows' ? cellValue(val.answer, 0) : 0n;
      const key = `content:${num}`;
      return {
        surfaces: [{ source: { width: 8, height: 8, content: key } as never, content: key }],
        fields: { v: { value: num, content: key } },
        motion: false,
      };
    },
  };
  let page!: Page<Fake>;
  let lane!: Lane<Fake>;
  page = new Page<Fake>({
    lanes: 1,
    post: (_lane, m: PageMessage) => {
      if (m.type === 'frame') lane.frame(m);
      else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
    },
  });
  lane = new Lane<Fake>({
    lane: 0, lanes: 1, views: () => view,
    crop: sources => {
      cropCall++;
      const make = () => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: source.width, close() { this.closed = true; } }));
      if (cropCall === 3) return new Promise<Fake[]>(resolve => { resolveHeldCrop = () => resolve(make()); });
      return Promise.resolve(make());
    },
    present: presentation => {
      if (delayNextPresentation) { delayNextPresentation = false; delayedPresentations.push(presentation); return; }
      page.offer(presentation);
    },
    release: () => {}, fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); }, later: () => {},
  });
  lane.follow(5, 'followed');
  lane.mount(1, [5], 'view');
  lane.bind(1, { generation: 1, surfaces: [{ width: 8, height: 8, dpr: 1 }], offset: 0, pin: null });
  page.mount(1, { route: ROUTE, lane: 0, surfaces: [{ install() {}, clear() {} }] });
  lane.bindRoute(5, ROUTE, NATIVE, 0);

  lane.receive('follow', message(100, [{ id: 5, hour: 100, answer: rows([1n], [111n]) }], false, NATIVE, 0));
  page.publish(100, { playing: false, seek: 0 });
  await flush();
  page.frame();
  assert.deepEqual([page.view(1).mode, page.view(1).h], ['picture', 100], 'hour 100 installs content A');

  delayNextPresentation = true;
  lane.receive('follow', message(105, [{ id: 5, hour: 105, answer: rows([1n], [155n]) }], false, NATIVE, 0));
  page.publish(105, { playing: false, seek: 0 });
  await flush();
  assert.equal(delayedPresentations.length, 1, 'hour-105 presentation is delayed in flight');

  lane.receive('follow', message(110, [{ id: 5, hour: 110, answer: rows([1n], [222n]) }], false, NATIVE, 0));
  page.publish(110, { playing: false, seek: 0 });
  await flush();
  assert.equal(cropCall, 3, 'hour-110 content B is suspended in crop');

  lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [111n]) }], false, NATIVE, 1));
  page.publish(10, { playing: false, seek: 1 });
  page.offer(delayedPresentations.shift()!);
  page.frame();
  page.publish(10, { playing: false, seek: 1 });
  assert.equal(page.view(1).mode, 'reading', 'Page cleared hour 100 on rewind and queued a stream refusal for delayed hour 105');

  resolveHeldCrop();
  await flush();
  await flush();
  page.frame();

  assert.equal(page.view(1).mode, 'picture', 'Page installs hour 10 on the first attempt even after a queued stream refusal flushes');
  assert.equal(page.view(1).h, 10);
  assert.equal(lane.stats.refused['kept content'] ?? 0, 0, 'must send full content A instead of SAME');
});

test('on-receipt acknowledgement time is charged once to the next serve budget, stretch, and served.ms', async () => {
  let clockMs = 0;
  let paintCostMs = 0;
  const presented: Presented<Fake>[] = [];
  const stretched: number[] = [];
  const served: { ms: number; changed: number }[] = [];
  const view: LaneView = {
    paint: (input): Painted => {
      clockMs += paintCostMs;
      return {
        surfaces: [],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
      };
    },
  };
  const lane = new Lane<Fake>({
    lane: 0, lanes: 1, views: () => view, crop: async () => [],
    clock: () => clockMs, budget: 10,
    present: presentation => { presented.push(presentation); },
    release: () => {},
    stretch: ms => { stretched.push(ms); },
    served: frame => { served.push({ ms: frame.ms, changed: frame.changed }); },
    fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); },
    later: () => {},
  });
  for (const [wid, sub] of [[1, 3], [2, 5], [3, 7]] as const) {
    lane.follow(sub, 'followed');
    lane.mount(wid, [sub], 'view');
    lane.bind(wid, { generation: 1, surfaces: [], offset: 0, pin: null });
    lane.bindRoute(sub, ROUTE, NATIVE, 0);
  }
  lane.receive('follow', message(10, [{ id: 3, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE, 0));
  lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
  await flush();
  const id10 = presented.at(-1)!.entries[0]!.id;

  lane.receive('follow', message(30, [
    { id: 5, hour: 30, answer: rows([1n], [30n]) },
    { id: 7, hour: 30, answer: rows([1n], [30n]) },
  ], false, NATIVE, 0));
  presented.length = 0;
  stretched.length = 0;
  served.length = 0;
  paintCostMs = 5;
  let ackTimed = false;
  const timedAck = {
    widget: 1,
    binding: 1,
    get id() {
      if (!ackTimed) { ackTimed = true; clockMs += 6; }
      return id10;
    },
  };
  lane.frame({
    type: 'frame', serial: 2, now: 30, target: 30, exact: true, seek: 0, speed: 0,
    installed: [timedAck], refused: [], inspect: [],
  });
  await flush();

  assert.equal(ackTimed, true, 'acknowledgement ran on receipt');
  assert.equal(presented.at(-1)!.entries.length, 1, '6 ms ack batch plus one 5 ms paint exhausts the 10 ms budget, deferring the second 5 ms widget');
  assert.equal(served.at(-1)!.ms, 11, 'served.ms includes the 6 ms ack work plus 5 ms paint');
  assert.equal(stretched.at(-1)!, 11, 'stretch receives the full 11 ms including on-receipt ack work');

  {
    let clock = 0;
    let paintMs = 30;
    const wakes: (() => void)[] = [];
    const preStretched: number[] = [];
    const preServed: { ms: number; cancelled?: boolean }[] = [];
    const prePresented: Presented<Fake>[] = [];
    const slowView: LaneView = {
      paint: (input): Painted => {
        clock += paintMs;
        return {
          surfaces: [],
          fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
          motion: false,
        };
      },
    };
    const l = new Lane<Fake>({
      lane: 0, lanes: 1, views: () => slowView, crop: async () => [],
      clock: () => clock, budget: 10,
      schedule: run => { wakes.push(run); },
      present: presentation => { prePresented.push(presentation); },
      release: () => {},
      stretch: ms => { preStretched.push(ms); },
      served: frame => { preServed.push({ ms: frame.ms, cancelled: frame.cancelled }); },
      fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); },
      later: () => {},
    });
    l.follow(5, 'followed');
    l.mount(1, [5], 'view');
    l.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
    l.bindRoute(5, ROUTE, NATIVE, 0);
    l.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE, 0));
    l.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await flush();
    const id1 = prePresented.at(-1)!.entries[0]!.id;
    assert.equal(wakes.length, 1, '30 ms serve opens hand-over');

    preStretched.length = 0;
    preServed.length = 0;
    paintMs = 0;
    let ticked = false;
    const slowAck = { widget: 1, binding: 1, get id() { if (!ticked) { ticked = true; clock += 6; } return id1; } };
    l.frame({ type: 'frame', serial: 2, now: 10, target: 10, exact: false, seek: 0, speed: 24, installed: [slowAck], refused: [], inspect: [] });
    l.stream(ROUTE, 1, 10);
    wakes.shift()!();
    await flush();
    assert.equal(preServed.at(-1)!.cancelled, true, 'pre-serve cancellation exit ran');
    assert.equal(preStretched.at(-1), 6, 'pre-serve cancellation exit stretches the 6 ms ack work');

    preStretched.length = 0;
    l.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE, 1));
    l.frame({ type: 'frame', serial: 3, now: 10, target: 10, exact: true, seek: 1, speed: 0, installed: [], refused: [], inspect: [] });
    await flush();
    assert.equal(preStretched.at(-1), 0, 'following ack-free serve receives no second charge');
  }

  {
    let clock = 0;
    let paintMs = 0;
    let resolveCrop!: () => void;
    let cropCall = 0;
    const postStretched: number[] = [];
    const postServed: { ms: number; cancelled?: boolean }[] = [];
    const postPresented: Presented<Fake>[] = [];
    const cropView: LaneView = {
      paint: (input): Painted => {
        clock += paintMs;
        return {
          surfaces: [{ source: { width: 4, height: 4, content: `h:${input.pictures[0]!.h}` } as never, content: `h:${input.pictures[0]!.h}` }],
          fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
          motion: false,
        };
      },
    };
    const l = new Lane<Fake>({
      lane: 0, lanes: 1, views: () => cropView,
      clock: () => clock, budget: 10,
      crop: sources => {
        cropCall++;
        const make = () => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: source.width, close() { this.closed = true; } }));
        if (cropCall === 2) return new Promise<Fake[]>(resolve => { resolveCrop = () => resolve(make()); });
        return Promise.resolve(make());
      },
      present: presentation => { postPresented.push(presentation); },
      release: () => {},
      stretch: ms => { postStretched.push(ms); },
      served: frame => { postServed.push({ ms: frame.ms, cancelled: frame.cancelled }); },
      fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); },
      later: () => {},
    });
    l.follow(5, 'followed');
    l.mount(1, [5], 'view');
    l.bind(1, { generation: 1, surfaces: [{ width: 4, height: 4, dpr: 1 }], offset: 0, pin: null });
    l.bindRoute(5, ROUTE, NATIVE, 0);
    l.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE, 0));
    l.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
    await flush();
    const id1 = postPresented.at(-1)!.entries[0]!.id;

    postStretched.length = 0;
    postServed.length = 0;
    paintMs = 4;
    let ticked = false;
    const slowAck = { widget: 1, binding: 1, get id() { if (!ticked) { ticked = true; clock += 6; } return id1; } };
    l.receive('follow', message(20, [{ id: 5, hour: 20, answer: rows([1n], [20n]) }], false, NATIVE, 0));
    l.frame({ type: 'frame', serial: 2, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [slowAck], refused: [], inspect: [] });
    await flush();
    assert.equal(cropCall, 2, 'second serve is suspended in crop');

    l.stream(ROUTE, 1, 20);
    resolveCrop();
    await flush();
    assert.equal(postServed.at(-1)!.cancelled, true, 'post-crop cancellation exit ran');
    assert.equal(postStretched.at(-1), 10, 'post-crop cancellation exit stretches the 6 ms ack plus 4 ms paint work');

    postStretched.length = 0;
    paintMs = 0;
    l.receive('follow', message(20, [{ id: 5, hour: 20, answer: rows([1n], [20n]) }], false, NATIVE, 1));
    l.frame({ type: 'frame', serial: 3, now: 20, target: 20, exact: true, seek: 1, speed: 0, installed: [], refused: [], inspect: [] });
    await flush();
    assert.equal(postStretched.at(-1), 0, 'following ack-free serve receives no second charge after post-crop abort');
  }
});

test('pinned widget repaints after post-crop abort triggered by another route seek', async () => {
  const ROUTE_B = 2, NATIVE_B = 10;
  let epoch = 1;
  let resolveCrop!: () => void;
  let cropCall = 0;
  const view: LaneView = {
    paint: (input): Painted => {
      const tag = `h:${input.pictures[0]!.h}:v${epoch}`;
      return {
        surfaces: [{ source: { width: 4, height: 4, content: tag } as never, content: tag }],
        fields: { v: { value: tag, content: tag } },
        motion: false,
      };
    },
  };
  let lane!: Lane<Fake>;
  const page = new Page<Fake>({
    lanes: 1,
    post: (_lane, m: PageMessage) => {
      if (m.type === 'frame') lane.frame(m);
      else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
    },
  });
  lane = new Lane<Fake>({
    lane: 0, lanes: 1, views: () => view,
    crop: sources => {
      cropCall++;
      const make = () => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: source.width, close() { this.closed = true; } }));
      if (cropCall === 2) return new Promise<Fake[]>(resolve => { resolveCrop = () => resolve(make()); });
      return Promise.resolve(make());
    },
    present: presentation => { page.offer(presentation); },
    release: () => {}, fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); }, later: () => {},
  });
  lane.follow(5, 'pinned');
  lane.follow(6, 'followed');
  lane.mount(1, [5], 'view');
  lane.mount(2, [6], 'view');
  lane.bind(1, { generation: 1, surfaces: [{ width: 4, height: 4, dpr: 1 }], offset: 0, pin: 10 });
  lane.bind(2, { generation: 1, surfaces: [{ width: 4, height: 4, dpr: 1 }], offset: 0, pin: null });
  page.mount(1, { route: ROUTE, lane: 0, pin: 10, surfaces: [{ install() {}, clear() {} }] });
  page.mount(2, { route: ROUTE_B, lane: 0, surfaces: [{ install() {}, clear() {} }] });
  lane.bindRoute(6, ROUTE_B, NATIVE_B, 0);

  lane.exact(5, 10, { val: 10 }, null);
  lane.receive('follow', message(10, [{ id: 6, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE_B, 0));
  page.publish(10, { playing: false, seek: 0 });
  await flush();
  page.frame();
  assert.equal(page.view(1).fields?.v, 'h:10:v1', 'pinned widget installs picture A');

  epoch = 2;
  lane.touch(1);
  await flush();
  assert.equal(cropCall, 2, 'picture B crop is suspended');

  lane.stream(ROUTE_B, 1, 20);
  lane.receive('follow', message(20, [{ id: 6, hour: 20, answer: rows([1n], [20n]) }], false, NATIVE_B, 1));
  resolveCrop();
  await flush();

  page.publish(20, { playing: false, seek: 1 });
  await flush();
  page.frame();
  assert.equal(page.view(1).fields?.v, 'h:10:v2', 'pinned widget repaints and installs picture B instead of staying on A');
});

test('data wake during hand-over ends hand-over immediately so a following speculative frame cannot drop the exact correction', async () => {
  const drainMicrotasks = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
  let clockMs = 0;
  let paintMs = 30;
  const wakes: (() => void)[] = [];
  const presented: Presented<Fake>[] = [];
  const view: LaneView = {
    paint: (input): Painted => {
      clockMs += paintMs;
      const val = input.pictures[0]!.parts[0]!.value;
      const num = val.kind === 'answer' && val.answer.kind === 'rows' ? Number(cellValue(val.answer, 0)) : 0;
      const key = `h:${input.pictures[0]!.h}:v:${num}`;
      return { surfaces: [], fields: { tag: { value: key, content: key } }, motion: false };
    },
  };
  const lane = new Lane<Fake>({
    lane: 0, lanes: 1, views: () => view, crop: async () => [],
    clock: () => clockMs, budget: 10,
    schedule: run => { wakes.push(run); },
    present: presentation => { presented.push(presentation); },
    release: () => {}, fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); }, later: () => {},
  });
  lane.follow(5, 'followed');
  lane.mount(1, [5], 'view');
  lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
  lane.bindRoute(5, ROUTE, NATIVE, 0);

  lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [100n]) }], false, NATIVE, 0));
  lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
  await drainMicrotasks();
  assert.equal(wakes.length, 1, '30 ms exact paint opens hand-over');
  assert.deepEqual(presented.map(p => p.entries[0]?.fields.tag), ['h:10:v:100']);

  paintMs = 2;
  lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [101n]) }], false, NATIVE, 0));
  await drainMicrotasks();
  lane.receive('follow', message(20, [{ id: 5, hour: 20, answer: rows([1n], [200n]) }], false, NATIVE, 0));
  lane.frame({ type: 'frame', serial: 2, now: 20, target: 20, exact: false, seek: 0, speed: 1440, installed: [], refused: [], inspect: [] });
  await drainMicrotasks();
  while (wakes.length) { wakes.shift()!(); await drainMicrotasks(); }

  assert.deepEqual(
    presented.map(p => p.entries[0]?.fields.tag),
    ['h:10:v:100', 'h:10:v:101', 'h:20:v:200'],
    'corrected hour-10 exact repaint from wake() must run before the later speculative hour-20 frame',
  );
});

test('invalidated MessageChannel wake from an interrupted hand-over cannot fire a later hand-over serve', async () => {
  const drainMicrotasks = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
  let clockMs = 0;
  const portQueue: unknown[] = [];
  const fakeChannel: WakeChannel = {
    port1: { onmessage: null },
    port2: { postMessage: data => { portQueue.push(data); } },
  };
  const schedule = macrotaskSchedule(() => fakeChannel)!;
  const presented: Presented<Fake>[] = [];
  const view: LaneView = {
    paint: (input): Painted => {
      clockMs += 30;
      return {
        surfaces: [],
        fields: { h: { value: input.pictures[0]!.h, content: `h:${input.pictures[0]!.h}` } },
        motion: false,
      };
    },
  };
  const lane = new Lane<Fake>({
    lane: 0, lanes: 1, views: () => view, crop: async () => [],
    clock: () => clockMs, budget: 10, schedule,
    present: presentation => { presented.push(presentation); },
    release: () => {}, fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); }, later: () => {},
  });
  lane.follow(5, 'followed');
  lane.mount(1, [5], 'view');
  lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
  lane.bindRoute(5, ROUTE, NATIVE, 0);
  lane.receive('follow', message(10, [{ id: 5, hour: 10, answer: rows([1n], [10n]) }], false, NATIVE, 0));
  lane.receive('follow', message(20, [{ id: 5, hour: 20, answer: rows([1n], [20n]) }], false, NATIVE, 0));
  lane.receive('follow', message(30, [{ id: 5, hour: 30, answer: rows([1n], [30n]) }], false, NATIVE, 0));

  lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
  await drainMicrotasks();
  assert.equal(portQueue.length, 1, 'wake A posted');

  lane.frame({ type: 'frame', serial: 2, now: 20, target: 20, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
  await drainMicrotasks();
  assert.equal(portQueue.length, 2, 'exact hour 20 interrupted wake A and posted wake B');

  lane.frame({ type: 'frame', serial: 3, now: 30, target: 30, exact: false, seek: 0, speed: 1440, installed: [], refused: [], inspect: [] });
  await drainMicrotasks();
  assert.deepEqual(presented.map(p => p.entries[0]?.h), [10, 20], 'speculative hour 30 waits for wake B');

  const wakeA = portQueue.shift();
  fakeChannel.port1.onmessage?.({ data: wakeA } as MessageEvent);
  await drainMicrotasks();
  assert.deepEqual(presented.map(p => p.entries[0]?.h), [10, 20], 'delivering stale wake A must not serve hour 30 before wake B arrives');

  const wakeB = portQueue.shift();
  fakeChannel.port1.onmessage?.({ data: wakeB } as MessageEvent);
  await drainMicrotasks();
  assert.deepEqual(presented.map(p => p.entries[0]?.h), [10, 20, 30], 'delivering wake B serves hour 30');
});

test('cropAll rejects on first failure while a sibling crop is held, serves exact hour 20 immediately, and closes the late sibling bitmap', async () => {
  const drainMicrotasks = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
  interface Tracked extends Fake { closeCount: number }
  let rejectFirst!: (err: Error) => void;
  let resolveSecond!: () => void;
  let lateBitmap: Tracked | null = null;
  let cropIndex = 0;
  const failedWidgets: [number, string][] = [];
  const view: LaneView = {
    paint: (input): Painted => {
      const h = input.pictures[0]!.h;
      const tag = `w${input.widget}:h${h}`;
      return {
        surfaces: [{ source: { width: 4, height: 4, content: tag } as never, content: tag }],
        fields: { tag: { value: tag, content: tag } },
        motion: false,
      };
    },
  };
  let lane!: Lane<Tracked>;
  const page = new Page<Tracked>({
    lanes: 1,
    post: (_lane, m: PageMessage) => {
      if (m.type === 'frame') lane.frame(m);
      else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
    },
  });
  lane = new Lane<Tracked>({
    lane: 0, lanes: 1, views: () => view,
    crop: sources => cropAll(sources, source => {
      const idx = ++cropIndex;
      const content = (source as unknown as { content: string }).content;
      const makeBitmap = (): Tracked => ({
        content, width: source.width, closed: false, closeCount: 0,
        close() { this.closeCount++; this.closed = true; },
      });
      if (idx === 1) return new Promise<Tracked>((_res, rej) => { rejectFirst = rej; });
      if (idx === 2) return new Promise<Tracked>(res => {
        resolveSecond = () => {
          lateBitmap = makeBitmap();
          res(lateBitmap);
        };
      });
      return Promise.resolve(makeBitmap());
    }),
    present: presentation => { page.offer(presentation); },
    release: () => {},
    fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); },
    failWidget: (widget, reason) => { failedWidgets.push([widget, reason]); page.name(widget, 'failed', reason); },
    later: () => {},
  });

  for (const [wid, sub] of [[1, 3], [2, 5], [3, 7]] as const) {
    lane.follow(sub, 'followed');
    lane.mount(wid, [sub], 'view');
    lane.bind(wid, { generation: 1, surfaces: [{ width: 4, height: 4, dpr: 1 }], offset: 0, pin: null });
    page.mount(wid, { route: ROUTE, lane: 0, surfaces: [{ install() {}, clear() {} }] });
    lane.bindRoute(sub, ROUTE, NATIVE, 0);
  }

  lane.receive('follow', message(10, [
    { id: 3, hour: 10, answer: rows([1n], [10n]) },
    { id: 5, hour: 10, answer: rows([1n], [10n]) },
  ], false, NATIVE, 0));
  page.publish(10, { playing: false, seek: 0 });
  await drainMicrotasks();
  assert.equal(cropIndex, 2, 'widgets 1 and 2 both started cropping at hour 10');

  lane.receive('follow', message(20, [
    { id: 7, hour: 20, answer: rows([1n], [20n]) },
  ], false, NATIVE, 0));
  page.publish(20, { playing: false, seek: 0 });
  rejectFirst(new Error('first crop failed'));
  await drainMicrotasks();
  page.frame();

  assert.deepEqual(failedWidgets.map(([w]) => w), [1, 2], 'failure is reported immediately on first rejection without waiting for sibling crop');
  assert.equal(page.view(3).mode, 'picture', 'third widget installs hour 20 immediately instead of waiting in reading');
  assert.equal(page.view(3).h, 20);

  resolveSecond();
  await drainMicrotasks();
  assert.ok(lateBitmap !== null, 'late sibling crop resolved');
  assert.equal((lateBitmap as Tracked).closeCount, 1, 'late-fulfilling sibling bitmap is closed exactly once');
});

test('seeded differential search: hand-over lane matches main at quiescence across 600 deterministic interleavings', async () => {
  const drainMicrotasks = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
  const ROUTE_A = 1, ROUTE_B = 2, NATIVE_B = 10;

  interface TrackedBitmap extends Fake { closeCount: number }
  interface PendingCrop { resolve: () => void; reject: (error: Error) => void }
  interface QueuedWake { data: unknown; stale: boolean }

  const makeRng = (seed: number) => {
    let s = seed >>> 0;
    return () => {
      s = (s + 0x6d2b79f5) >>> 0;
      let t = Math.imul(s ^ (s >>> 15), 1 | s);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  };

  const makeSurface = () => {
    let held: TrackedBitmap | null = null;
    return {
      install(bitmap: TrackedBitmap) {
        if (held && held !== bitmap) held.close();
        held = bitmap;
      },
      clear() {
        if (held) { held.close(); held = null; }
      },
    };
  };

  const runSeed = async (seed: number) => {
    const rng = makeRng(seed);
    const randInt = (n: number) => Math.floor(rng() * n);
    const log: string[] = [];
    const failWith = (msg: string): never => {
      assert.fail(`seed=${seed} ${msg}\nevents:\n${log.join('\n')}`);
    };

    let visualEpoch = 1;
    let paintDurationMs = 12;
    let holdNextCrop = false;

    const createSide = (withHandover: boolean) => {
      let clockMs = 0;
      let inCrop = false;
      let concurrentViolation = false;
      let serveCount = 0;
      let exactServeCount = 0;
      let cropCount = 0;
      let offerCount = 0;
      let lastExactSeek: number | null = null;
      let pendingSpeculative = false;
      let lastHit1: string | null = null;
      let lastHit2: string | null = null;
      const bitmaps: TrackedBitmap[] = [];
      const crops: PendingCrop[] = [];
      const bufferedOffers: Presented<TrackedBitmap>[] = [];
      const wakes: QueuedWake[] = [];
      const laters: (() => void)[] = [];
      let delayOffers = false;
      const surfaces = [makeSurface(), makeSurface()];
      const fakeChannel: WakeChannel = {
        port1: { onmessage: null },
        port2: {
          postMessage: data => {
            for (const queued of wakes) queued.stale = true;
            wakes.push({ data, stale: false });
          },
        },
      };
      const schedule = withHandover ? macrotaskSchedule(() => fakeChannel) : undefined;
      let lane!: Lane<TrackedBitmap>;
      const page = new Page<TrackedBitmap>({
        lanes: 1,
        post: (_lane, m: PageMessage) => {
          if (m.type === 'frame') {
            if (m.exact) { lastExactSeek = m.seek; pendingSpeculative = false; }
            else { lastExactSeek = null; pendingSpeculative = true; }
            lane.frame(m);
          } else if (m.type === 'stream') {
            if (lastExactSeek !== null && lastExactSeek < m.seek) lastExactSeek = null;
            lane.stream(m.route, m.seek, m.target);
          }
        },
      });
      const view: LaneView = {
        paint: (input): Painted => {
          if (inCrop) {
            concurrentViolation = true;
            failWith(`${withHandover ? 'handover' : 'main'} started paint while a crop was still in progress`);
          }
          clockMs += paintDurationMs;
          const val = input.pictures[0]!.parts[0]!.value;
          const num = val.kind === 'answer' && val.answer.kind === 'rows'
            ? Number(cellValue(val.answer, 0))
            : val.kind === 'panel' && val.data && typeof val.data === 'object' && 'val' in val.data
              ? Number((val.data as { val: number }).val)
              : 0;
          const tag = input.widget === 1 ? `w1:${input.pictures[0]!.h}:val:${num}:v${visualEpoch}` : `w2:${input.pictures[0]!.h}:val:${num}`;
          return {
            surfaces: [{ source: { width: 4, height: 4, content: tag } as never, content: tag }],
            fields: { tag: { value: tag, content: tag } },
            motion: false,
            hit: () => tag,
          };
        },
      };
      lane = new Lane<TrackedBitmap>({
        lane: 0, lanes: 1, views: () => view,
        clock: () => clockMs, budget: 10,
        ...schedule ? { schedule } : {},
        crop: sources => {
          if (inCrop) {
            concurrentViolation = true;
            failWith(`${withHandover ? 'handover' : 'main'} started a second crop while one was still in progress`);
          }
          cropCount++;
          const makeBitmaps = () => sources.map(source => {
            const bm: TrackedBitmap = {
              content: (source as unknown as { content: string }).content,
              width: source.width,
              closed: false,
              closeCount: 0,
              close() {
                this.closeCount++;
                this.closed = true;
                if (this.closeCount > 1) failWith(`${withHandover ? 'handover' : 'main'} closed bitmap ${this.content} ${this.closeCount} times`);
              },
            };
            bitmaps.push(bm);
            return bm;
          });
          if (holdNextCrop) {
            inCrop = true;
            return new Promise<TrackedBitmap[]>((resolve, reject) => {
              crops.push({
                resolve: () => { inCrop = false; resolve(makeBitmaps()); },
                reject: err => { inCrop = false; reject(err); },
              });
            });
          }
          return Promise.resolve(makeBitmaps());
        },
        present: presentation => {
          offerCount++;
          if (delayOffers) bufferedOffers.push(presentation);
          else page.offer(presentation);
        },
        release: () => {},
        served: frame => {
          serveCount++;
          if (!frame.exact) pendingSpeculative = false;
          if (frame.exact && !frame.cancelled) exactServeCount++;
        },
        hit: (widget, key) => {
          if (widget === 1) lastHit1 = key;
          else if (widget === 2) lastHit2 = key;
        },
        fail: reason => { failWith(`${withHandover ? 'handover' : 'main'} lane failure: ${reason}`); },
        failWidget: () => {},
        later: fn => { laters.push(fn); },
      });
      lane.follow(5, 'pinned');
      lane.follow(6, 'followed');
      lane.mount(1, [5], 'view');
      lane.mount(2, [6], 'view');
      lane.bind(1, { generation: 1, surfaces: [{ width: 4, height: 4, dpr: 1 }], offset: 0, pin: 10 });
      lane.bind(2, { generation: 1, surfaces: [{ width: 4, height: 4, dpr: 1 }], offset: 0, pin: null });
      page.mount(1, { route: ROUTE_A, lane: 0, pin: 10, surfaces: [surfaces[0]!] });
      page.mount(2, { route: ROUTE_B, lane: 0, surfaces: [surfaces[1]!] });
      lane.bindRoute(6, ROUTE_B, NATIVE_B, 0);

      return {
        lane, page, crops, bufferedOffers, wakes, laters, fakeChannel, bitmaps, surfaces,
        onStream(seek: number) { if (lastExactSeek !== null && lastExactSeek < seek) lastExactSeek = null; },
        get hasEligibleExactWake() { return lastExactSeek !== null && !pendingSpeculative; },
        get delayOffers() { return delayOffers; },
        set delayOffers(v: boolean) { delayOffers = v; },
        get inCrop() { return inCrop; },
        get concurrentViolation() { return concurrentViolation; },
        get serveCount() { return serveCount; },
        get exactServeCount() { return exactServeCount; },
        get cropCount() { return cropCount; },
        get offerCount() { return offerCount; },
        get lastHit1() { return lastHit1; },
        get lastHit2() { return lastHit2; },
      };
    };

    const main = createSide(false);
    const over = createSide(true);
    const sides = [main, over] as const;

    let seekB = 0;
    let currentHour = 10;
    let fact5Val = 100;
    const fact6BySeekHour = new Map<string, number>();
    const setFact6 = (seek: number, hour: number, val: number) => {
      fact6BySeekHour.set(`${seek}:${hour}`, val);
    };
    const pushFact5 = (val: number) => {
      fact5Val = val;
      for (const s of sides) s.lane.exact(5, 10, { val }, null);
    };
    const pushFact6 = (seek: number, hour: number, val: number) => {
      setFact6(seek, hour, val);
      const bytes = message(hour, [{ id: 6, hour, answer: rows([1n], [BigInt(val)]) }], false, NATIVE_B, seek);
      for (const s of sides) s.lane.receive('follow', bytes);
    };

    pushFact5(100);
    pushFact6(0, 10, 200);
    for (const s of sides) s.page.publish(10, { playing: false, seek: 0 });
    await drainMicrotasks();
    for (const s of sides) s.page.frame();
    await drainMicrotasks();
    while (over.wakes.length) {
      const w = over.wakes.shift()!;
      over.fakeChannel.port1.onmessage?.({ data: w.data } as MessageEvent);
      await drainMicrotasks();
    }

    const steps = 22;
    for (let step = 0; step < steps; step++) {
      paintDurationMs = rng() < 0.6 ? 14 : 3;
      holdNextCrop = rng() < 0.35;
      const delay = rng() < 0.35;
      main.delayOffers = delay;
      over.delayOffers = delay;

      const choice = randInt(10);
      if (choice === 0) {
        currentHour += 10;
        const val = 200 + step * 10 + randInt(5);
        pushFact6(seekB, currentHour, val);
        log.push(`${step}: spec(h=${currentHour},val=${val},ms=${paintDurationMs},hold=${holdNextCrop})`);
        for (const s of sides) s.page.publish(currentHour, { playing: true, step: 10, speed: 240 });
        await drainMicrotasks();
      } else if (choice === 1) {
        const hour = rng() < 0.5 ? currentHour : (currentHour = 10 + randInt(5) * 10);
        const val = 300 + step * 10;
        const overWasInHandover = !over.inCrop && over.wakes.some(w => !w.stale);
        const overBefore = over.exactServeCount;
        pushFact6(seekB, hour, val);
        log.push(`${step}: exact(h=${hour},val=${val},ms=${paintDurationMs},hold=${holdNextCrop})`);
        for (const s of sides) s.page.publish(hour, { playing: false, seek: seekB });
        await drainMicrotasks();
        if (overWasInHandover && !over.inCrop && !holdNextCrop && over.exactServeCount === overBefore) {
          failWith(`incoming exact frame at step ${step} was held in handover instead of serving immediately`);
        }
      } else if (choice === 2) {
        const val = 500 + step * 10;
        const mainBefore = main.exactServeCount;
        const overBefore = over.exactServeCount;
        const neitherInCrop = !main.inCrop && !over.inCrop;
        const overEligibleExact = over.hasEligibleExactWake;
        const targetSub = rng() < 0.5 ? 5 : 6;
        if (targetSub === 5) {
          log.push(`${step}: facts5(val=${val},ms=${paintDurationMs},hold=${holdNextCrop})`);
          pushFact5(val);
        } else {
          log.push(`${step}: facts6(seek=${seekB},h=${currentHour},val=${val},ms=${paintDurationMs},hold=${holdNextCrop})`);
          pushFact6(seekB, currentHour, val);
        }
        await drainMicrotasks();
        if (overEligibleExact && neitherInCrop && !holdNextCrop && main.exactServeCount > mainBefore && over.exactServeCount === overBefore) {
          failWith(`exact data wake at step ${step} ran on main but was held/lost in handover`);
        }
      } else if (choice === 3) {
        visualEpoch++;
        log.push(`${step}: touchPinned(v=${visualEpoch},ms=${paintDurationMs},hold=${holdNextCrop})`);
        for (const s of sides) s.lane.touch(1);
        await drainMicrotasks();
      } else if (choice === 4) {
        seekB++;
        currentHour = 10 + randInt(3) * 10;
        const val = 700 + seekB * 10;
        const pageSeek = rng() < 0.5;
        log.push(`${step}: seekB(seek=${seekB},pageSeek=${pageSeek},h=${currentHour},val=${val},ms=${paintDurationMs},hold=${holdNextCrop})`);
        if (pageSeek) {
          for (const s of sides) s.page.publish(currentHour, { playing: false, seek: seekB });
          pushFact6(seekB, currentHour, val);
        } else {
          for (const s of sides) { s.onStream(seekB); s.lane.stream(ROUTE_B, seekB, currentHour); }
          pushFact6(seekB, currentHour, val);
        }
        await drainMicrotasks();
      } else if (choice === 5) {
        log.push(`${step}: deliverOffers(m=${main.bufferedOffers.length},o=${over.bufferedOffers.length})`);
        for (const s of sides) {
          if (s.bufferedOffers.length) s.page.offer(s.bufferedOffers.shift()!);
          s.page.frame();
        }
        await drainMicrotasks();
      } else if (choice === 6) {
        log.push(`${step}: ackFrame`);
        for (const s of sides) s.page.frame();
        await drainMicrotasks();
      } else if (choice === 7) {
        const reject = rng() < 0.15;
        log.push(`${step}: crop(${reject ? 'reject' : 'resolve'},m=${main.crops.length},o=${over.crops.length})`);
        for (const s of sides) {
          if (!s.crops.length) continue;
          const pending = s.crops.shift()!;
          if (reject) pending.reject(new Error('injected crop failure'));
          else pending.resolve();
        }
        await drainMicrotasks();
        if (reject) {
          for (const s of sides) {
            for (const wid of [1, 2]) {
              s.page.rebind(wid, { subscription: true });
              const binding = s.page.widgets.get(wid)!.binding;
              s.lane.bind(wid, { generation: binding, surfaces: [{ width: 4, height: 4, dpr: 1 }], offset: 0, pin: wid === 1 ? 10 : null }, undefined, true);
            }
          }
          await drainMicrotasks();
        }
      } else if (choice === 8) {
        if (over.wakes.length) {
          const w = over.wakes.shift()!;
          const servedBefore = over.serveCount;
          const cropsBefore = over.cropCount;
          log.push(`${step}: deliverWake(stale=${w.stale},remaining=${over.wakes.length})`);
          over.fakeChannel.port1.onmessage?.({ data: w.data } as MessageEvent);
          await drainMicrotasks();
          if (w.stale && (over.serveCount !== servedBefore || over.cropCount !== cropsBefore)) {
            failWith(`stale wake fired a serve at step ${step} (served ${servedBefore}->${over.serveCount}, crops ${cropsBefore}->${over.cropCount})`);
          }
        }
      } else {
        log.push(`${step}: pointer`);
        for (const s of sides) {
          s.lane.pointer(1, 0, [1, 1]);
          s.lane.pointer(2, 0, [1, 1]);
        }
      }
    }

    holdNextCrop = false;
    paintDurationMs = 2;
    main.delayOffers = false;
    over.delayOffers = false;

    const drainBoth = async () => {
      for (let round = 0; round < 30; round++) {
        let progressed = false;
        const beforeOffers = main.offerCount + over.offerCount;
        const beforeServes = main.serveCount + over.serveCount;
        for (const s of sides) {
          while (s.crops.length) { s.crops.shift()!.resolve(); progressed = true; }
          while (s.bufferedOffers.length) { s.page.offer(s.bufferedOffers.shift()!); progressed = true; }
          while (s.laters.length) { s.laters.shift()!(); progressed = true; }
        }
        while (over.wakes.length) {
          const w = over.wakes.shift()!;
          over.fakeChannel.port1.onmessage?.({ data: w.data } as MessageEvent);
          progressed = true;
        }
        await drainMicrotasks();
        for (const s of sides) s.page.frame();
        await drainMicrotasks();
        const afterOffers = main.offerCount + over.offerCount;
        const afterServes = main.serveCount + over.serveCount;
        if (afterOffers > beforeOffers || afterServes > beforeServes) progressed = true;
        if (!progressed && !main.crops.length && !over.crops.length && !over.wakes.length && !main.laters.length && !over.laters.length && !main.bufferedOffers.length && !over.bufferedOffers.length) break;
      }
    };

    await drainBoth();
    const mainServesBeforeSpecProbe = main.serveCount;
    const overServesBeforeSpecProbe = over.serveCount;
    for (const s of sides) {
      s.lane.frame({
        type: 'frame', serial: 900000 + seed, now: currentHour, target: currentHour,
        exact: false, seek: seekB, speed: 240, installed: [], refused: [], inspect: [],
      });
    }
    await drainMicrotasks();
    if (main.serveCount <= mainServesBeforeSpecProbe || over.serveCount <= overServesBeforeSpecProbe) {
      failWith(`lane stranded working with nothing scheduled after first drain (main ${mainServesBeforeSpecProbe}->${main.serveCount}, over ${overServesBeforeSpecProbe}->${over.serveCount})`);
    }
    await drainBoth();

    const finalHour = currentHour + 10;
    const finalVal6 = 9000 + seed;
    pushFact6(seekB, finalHour, finalVal6);
    for (const s of sides) {
      s.page.publish(finalHour, { playing: false, seek: seekB });
    }
    await drainBoth();

    const expected1 = `w1:10:val:${fact5Val}:v${visualEpoch}`;
    const expected2 = `w2:${finalHour}:val:${finalVal6}`;

    for (const wid of [1, 2]) {
      const vm = main.page.view(wid);
      const vo = over.page.view(wid);
      if (vm.mode !== 'picture' || vo.mode !== 'picture') {
        failWith(`quiescence widget ${wid} not picture: main=${vm.mode} over=${vo.mode}`);
      }
      if (vm.h !== vo.h || vm.fields?.tag !== vo.fields?.tag) {
        failWith(`quiescence widget ${wid} mismatch: main=(h:${vm.h},tag:${vm.fields?.tag}) over=(h:${vo.h},tag:${vo.fields?.tag})`);
      }
      const expected = wid === 1 ? expected1 : expected2;
      if (vo.fields?.tag !== expected || vm.fields?.tag !== expected) {
        failWith(`quiescence widget ${wid} stale vs oracle: expected=${expected} main=${vm.fields?.tag} over=${vo.fields?.tag}`);
      }
    }

    for (const s of sides) {
      s.lane.pointer(1, 0, [1, 1]);
      s.lane.pointer(2, 0, [1, 1]);
    }
    if (main.lastHit1 !== over.lastHit1 || main.lastHit2 !== over.lastHit2 || over.lastHit1 !== expected1 || over.lastHit2 !== expected2) {
      failWith(`quiescence pointer mismatch: main=(${main.lastHit1},${main.lastHit2}) over=(${over.lastHit1},${over.lastHit2}) expected=(${expected1},${expected2})`);
    }

    const servesBeforeProbe = over.serveCount;
    visualEpoch++;
    over.lane.touch(1);
    await drainMicrotasks();
    if (over.serveCount <= servesBeforeProbe) {
      failWith(`handover lane stuck working with nothing scheduled at quiescence`);
    }
    await drainBoth();

    if (main.concurrentViolation || over.concurrentViolation) {
      failWith(`concurrent serve detected`);
    }

    for (const s of sides) {
      s.page.unmount(1);
      s.page.unmount(2);
      for (const surface of s.surfaces) surface.clear();
      for (const bm of s.bitmaps) {
        if (bm.closeCount !== 1) {
          failWith(`bitmap ${bm.content} closeCount=${bm.closeCount} (expected 1)`);
        }
      }
    }
  };

  for (let seed = 1; seed <= 600; seed++) {
    await runSeed(seed);
  }
});

test('payloads are taken in on every message, not at paint: two binds between paints keep every tile, and each tile crosses once', async () => {
  const seen: string[][] = [];
  const tiles: LaneView = { paint: input => {
    const lines = (input.model as { tiledBySource: { main: Record<string, { tag: string }> } }).tiledBySource.main;
    seen.push(Object.values(lines).map(line => line.tag));
    return { surfaces: [], fields: {}, motion: false };
  } };
  const { lane, mount, step } = rig({ tiles });
  let crossed = 0;
  const sender = new PayloadSender(message => { crossed += message.put?.length ?? 0; lane.payloads(structuredClone(message)); });
  mount(1, [], 'tiles', 0, { dated: { h: 0, c: 100 } });
  const ger = { tag: 'GER', buckets: [1, 2, 3] };
  const ita = { tag: 'ITA', buckets: [4, 5, 6] };
  const bind = (lines: Record<string, unknown>) => {
    const encoded = sender.acquire({ family: 'chart', kind: 'test', tiledBySource: { main: lines } });
    lane.bind(1, { generation: 2, surfaces: [], offset: 0, pin: null, dated: { h: 0, c: 100 } }, structuredClone(encoded.wire));
    return encoded.ids;
  };
  const first = bind({ GER: ger });
  const second = bind({ GER: ger, ITA: ita });
  sender.release(first);
  await step(10, { playing: false });
  assert.deepEqual(seen.at(-1), ['GER', 'ITA'], 'the tile the first bind brought is still there for the second');
  assert.equal(crossed, 2, 'each tile crossed once');
  sender.release(second);
});

test('a keyed container holding a scalar crosses whole, so 0, 1, null and an empty panel decode apart, and object entries still cross one by one', () => {
  const store = new PayloadStore();
  let crossed = 0;
  const sender = new PayloadSender(message => { crossed += message.put?.length ?? 0; store.apply(structuredClone(message)); });
  const decoded = (staticPanels: object) => (store.resolve(structuredClone(sender.acquire({ staticPanels }).wire)) as { staticPanels: unknown }).staticPanels;
  assert.deepEqual([{ main: 0 }, { main: 1 }, { main: null }, {}].map(decoded), [{ main: 0 }, { main: 1 }, { main: null }, {}]);
  const a = { rows: [1] };
  const b = { rows: [2] };
  decoded({ main: a, other: b });
  const before = crossed;
  assert.deepEqual(decoded({ main: a, other: { rows: [3] } }), { main: a, other: { rows: [3] } });
  assert.equal(crossed - before, 1, 'only the changed entry crosses');
});

test('a date-only supply for a stamp the lane does not hold is dropped, and a new owner clears the previous owner’s statics', async () => {
  const seen: { panels: unknown; dated: unknown }[] = [];
  const watch: LaneView = { paint: input => {
    seen.push({ panels: (input.model as { staticPanels?: unknown } | undefined)?.staticPanels ?? null, dated: input.binding.dated ?? null });
    return { surfaces: [], fields: {}, motion: false };
  } };
  const { lane, mount, step } = rig({ watch });
  mount(1, [], 'watch', 0, { dated: { h: 0, c: 100 } });
  lane.bind(1, { generation: 2, owner: 'a', surfaces: [], offset: 0, pin: null, dated: { h: 0, c: 100 } }, { family: 'f', kind: 'k' });
  lane.supply(1, { stamp: 5, statics: { staticPanels: { main: { at: 10 } } }, dated: { h: 10, c: 20 } });
  lane.supply(1, { stamp: 4, dated: { h: 0, c: 100 } });
  await step(15, { playing: false });
  assert.deepEqual(seen.at(-1), { panels: { main: { at: 10 } }, dated: { h: 10, c: 20 } }, 'dates stamped for another answer never date this one');
  lane.supply(1, { stamp: 5, dated: { h: 10, c: 30 } });
  await step(25, { playing: false });
  assert.deepEqual(seen.at(-1)?.dated, { h: 10, c: 30 }, 'dates for the answer held are applied');
  lane.bind(1, { generation: 3, owner: 'b', surfaces: [], offset: 0, pin: null, dated: { h: 0, c: 100 } });
  await step(26, { playing: false });
  assert.equal(seen.at(-1)?.panels, null, 'the previous owner’s panels are gone');
});

test('R5-S6, R2-S12: before mount, a statics replacement takes its own stamp, undefined on a clear, and a date for another stamp is dropped', async () => {
  const seen = new Map<number, { panels: unknown; dated: unknown }>();
  const watch: LaneView = { paint: input => {
    seen.set(input.widget, { panels: (input.model as { staticPanels?: unknown } | undefined)?.staticPanels ?? null, dated: input.binding.dated ?? null });
    return { surfaces: [], fields: {}, motion: false };
  } };
  const { lane, mount, step } = rig({ watch });
  lane.supply(1, { stamp: 5, statics: { staticPanels: { main: { at: 10 } } }, dated: { h: 10, c: 20 } });
  lane.supply(1, { statics: null });
  mount(1, [], 'watch', 0, { dated: { h: 0, c: 30 } });
  lane.bind(1, { generation: 2, surfaces: [], offset: 0, pin: null, dated: { h: 0, c: 30 } }, { family: 'f', kind: 'k' });
  lane.supply(1, { dated: { h: 0, c: 100 } });
  await step(50, { playing: false });
  assert.deepEqual(seen.get(1), { panels: null, dated: { h: 0, c: 100 } }, 'R5-S6: after a clear, an unstamped date extension applies');

  lane.supply(2, { stamp: 5, statics: { staticPanels: { main: { at: 10 } } }, dated: { h: 10, c: 20 } });
  lane.supply(2, { stamp: 4, dated: { h: 0, c: 100 } });
  mount(2, [], 'watch', 0, { dated: { h: 10, c: 20 } });
  lane.bind(2, { generation: 2, surfaces: [], offset: 0, pin: null, dated: { h: 10, c: 20 } }, { family: 'f', kind: 'k' });
  lane.supply(2, { stamp: 5, dated: { h: 10, c: 30 } });
  await step(25, { playing: false });
  assert.deepEqual(seen.get(2), { panels: { main: { at: 10 } }, dated: { h: 10, c: 30 } }, 'R2-S12: the buffered panels keep their own stamp, so their own dates still apply');
});

test('a view that is waiting presents nothing and is not painted again until it wakes the widget; a family switch disposes the outgoing view', async () => {
  let paints = 0, ready = false;
  const unmounted: string[] = [];
  const states: Map<string, unknown>[] = [];
  const others: Map<string, unknown>[] = [];
  const loading: LaneView = {
    paint: (_input, state): Painted => { paints++; states.push(state); return ready ? { surfaces: [], fields: { at: { value: 1, content: 'one' } }, motion: false } : { surfaces: [], fields: {}, motion: false, waiting: true }; },
    unmount: () => { unmounted.push('loading'); },
  };
  const other: LaneView = { paint: (_input, state) => { others.push(state); return { surfaces: [], fields: {}, motion: false }; } };
  const { page, lane, mount, step } = rig({ loading, other });
  mount(1, [], 'loading', 0, { dated: { h: 0, c: 100 } });
  for (let hour = 10; hour < 15; hour++) await step(hour, { playing: false });
  assert.equal(paints, 1, 'one paint while the view waits, not one per frame');
  assert.notEqual(page.view(1).mode, 'picture');
  ready = true;
  lane.touch(1);
  await step(15, { playing: false });
  assert.equal(paints, 2, 'its own wake paints it again');
  lane.mount(1, [], 'other');
  lane.bind(1, { generation: 2, surfaces: [], offset: 0, pin: null, dated: { h: 0, c: 100 } });
  await step(16, { playing: false });
  assert.deepEqual(unmounted, ['loading'], 'the outgoing view is disposed');
  assert.ok(others.length > 0, 'the new view paints');
  assert.ok(others.every(state => !states.includes(state)), 'the new view starts with its own state');
});

test('N widgets on a lane: when one changes (including same-owner rebind on unchanged widgets and a zero-sized off-screen surface), exactly one crop runs', async () => {
  const cropped: number[] = [];
  const view: LaneView = {
    paint(input): Painted {
      const surface = input.binding.surfaces[0];
      if (!surface || surface.width <= 0 || surface.height <= 0) {
        return { surfaces: [], fields: {}, motion: false, waiting: true };
      }
      const val = input.pictures[0]?.parts[0]?.value;
      const num = val?.kind === 'answer' && val.answer.kind === 'rows' ? String(cellValue(val.answer, 0)) : 'none';
      const tag = `w${input.widget}:${num}`;
      return {
        surfaces: [{ source: { width: surface.width, height: surface.height, widget: input.widget, content: tag } as never, content: tag }],
        fields: { v: { value: num, content: tag } },
        motion: false,
      };
    },
  };
  let lane!: Lane<Fake>;
  const page = new Page<Fake>({
    lanes: 1,
    post: (_lane, m: PageMessage) => {
      if (m.type === 'frame') lane.frame(m);
      else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target);
    },
  });
  lane = new Lane<Fake>({
    lane: 0,
    lanes: 1,
    clock: () => 0,
    views: () => view,
    crop: async sources => sources.map((source): Fake => {
      const meta = source as unknown as { widget: number; content: string };
      cropped.push(meta.widget);
      return { content: meta.content, width: source.width, closed: false, close() { this.closed = true; } };
    }),
    present: presentation => { page.offer(presentation); },
    release: () => {},
    fail: reason => { throw new Error(reason); },
    later: () => {},
  });
  for (let i = 1; i <= 5; i++) {
    const sub = i * 10;
    lane.follow(sub, 'followed');
    lane.mount(i, [sub], 'view');
    lane.bind(i, { generation: 1, owner: `o${i}`, surfaces: [{ width: 100, height: 40, dpr: 1 }], offset: 0, pin: null });
    page.mount(i, { route: ROUTE, lane: 0, owner: `o${i}`, surfaces: [{ install() {}, clear() {} }] });
    lane.bindRoute(sub, ROUTE, NATIVE, 0);
  }
  lane.follow(60, 'followed');
  lane.mount(6, [60], 'view');
  lane.bind(6, { generation: 1, owner: 'o6', surfaces: [{ width: 0, height: 0, dpr: 1 }], offset: 0, pin: null });
  page.mount(6, { route: ROUTE, lane: 0, owner: 'o6', surfaces: [{ install() {}, clear() {} }] });
  lane.bindRoute(60, ROUTE, NATIVE, 0);

  lane.receive('follow', message(10, [1, 2, 3, 4, 5, 6].map(i => ({ id: i * 10, hour: 10, answer: rows([1n], [BigInt(i)]) })), false, NATIVE, 0));
  page.publish(10, { playing: false, seek: 0 });
  await flush();
  page.frame();
  assert.equal(cropped.length, 5, 'initial frame crops the 5 visible widgets and 0 for the zero-sized widget');
  assert.notEqual(page.view(6).mode, 'picture', 'zero-sized widget waits without presenting an empty picture');
  cropped.length = 0;

  page.rebind(2, { owner: 'o2' });
  lane.bind(2, { generation: 2, owner: 'o2', surfaces: [{ width: 100, height: 40, dpr: 1 }], offset: 0, pin: null });
  lane.receive('follow', message(20, [{ id: 30, hour: 20, answer: rows([1n], [999n]) }], false, NATIVE, 0));
  page.publish(20, { playing: false, seek: 0 });
  await flush();
  page.frame();
  assert.deepEqual(cropped, [3], 'only the one changed widget crops; unchanged widgets and same-owner rebind cost 0 crops');
  assert.equal(page.view(2).mode, 'picture', 'same-owner rebind keeps the installed picture without re-cropping');
  cropped.length = 0;

  const replacement = { installs: 0, install() { this.installs++; }, clear() {} };
  page.remount(2, [replacement]);
  lane.bind(2, { generation: 3, owner: 'o2', surfaces: [{ width: 100, height: 40, dpr: 1 }], offset: 0, pin: null }, undefined, false, true);
  page.publish(20, { playing: false, seek: 0 });
  await flush();
  page.frame();
  assert.deepEqual(cropped, [2], 'remounting an equal-size page surface forces a fresh crop without kept-content refusal');
  assert.equal(page.view(2).mode, 'picture');
  assert.equal(replacement.installs, 1);
  cropped.length = 0;

  page.rebind(2, { owner: 'o2-next' });
  lane.bind(2, { generation: 4, owner: 'o2-next', surfaces: [{ width: 100, height: 40, dpr: 1 }], offset: 0, pin: null });
  page.publish(20, { playing: false, seek: 0 });
  await flush();
  page.frame();
  assert.deepEqual(cropped, [2], 'changing owner forces a fresh crop');
});

test('cropSurfaces transfers registered canvases synchronously with zero copies, restores on recovery re-crop, handles duplicates, and cleans up on error', async () => {
  interface MockBmp { pixels: string; width: number; closed: boolean; close(): void }
  interface MockCanvas {
    width: number;
    height: number;
    pixels: string | null;
    transfers: number;
    transferToImageBitmap(): MockBmp;
  }
  const makeCanvas = (initial: string): MockCanvas => {
    const c: MockCanvas = {
      width: 64,
      height: 32,
      pixels: initial,
      transfers: 0,
      transferToImageBitmap() {
        this.transfers++;
        if (this.pixels === null) throw new Error('blank canvas transferred');
        const pixels = this.pixels;
        this.pixels = null;
        return { pixels, width: this.width, closed: false, close() { this.closed = true; } };
      },
    };
    return c;
  };

  let fallbacks = 0;
  const fallback = async (source: { width: number; height: number }): Promise<MockBmp> => {
    fallbacks++;
    const c = source as unknown as MockCanvas;
    if (c.pixels === null) throw new Error('blank canvas in fallback');
    const bmp: MockBmp = { pixels: c.pixels, width: c.width, closed: false, close() { bmp.closed = true; } };
    return bmp;
  };

  const c1 = makeCanvas('v1');
  let restores = 0;
  markCanvasPainted(c1, () => { restores++; c1.pixels = 'v1'; });

  const b1 = await cropSurfaces<MockBmp>([c1], fallback);
  assert.equal(b1[0]!.pixels, 'v1', 'first transfer yields painted pixels');
  assert.equal(c1.transfers, 1, 'uses transferToImageBitmap');
  assert.equal(fallbacks, 0, 'no async createImageBitmap fallback on registered canvas');
  assert.equal(restores, 0, 'no restore needed on freshly painted canvas');
  assert.equal(c1.pixels, null, 'transferToImageBitmap detached the backing buffer');

  const b2 = await cropSurfaces<MockBmp>([c1], fallback);
  assert.equal(b2[0]!.pixels, 'v1', 'recovery re-crop restores pixels before transferring');
  assert.equal(restores, 1, 'restore ran once for recovery re-crop');
  assert.equal(c1.transfers, 2, 'still transfers without async fallback');
  assert.equal(fallbacks, 0);

  c1.pixels = 'v2';
  markCanvasPainted(c1, () => { restores++; c1.pixels = 'v2'; });
  const dup = await cropSurfaces<MockBmp>([c1, c1], fallback);
  assert.deepEqual(dup.map(b => b.pixels), ['v2', 'v2'], 'duplicate source in one batch produces two valid bitmaps');
  assert.equal(fallbacks, 1, 'earlier duplicate uses fallback so canvas stays intact for final transfer');
  assert.equal(c1.transfers, 3, 'final duplicate uses zero-copy transfer');

  const unregistered = makeCanvas('raw');
  const rawRes = await cropSurfaces<MockBmp>([unregistered], fallback);
  assert.equal(rawRes[0]!.pixels, 'raw');
  assert.equal(unregistered.transfers, 0, 'unregistered canvas is never destructively transferred');
  assert.equal(unregistered.pixels, 'raw', 'unregistered canvas retains its pixels');
  assert.equal(fallbacks, 2);

  const ok = makeCanvas('ok');
  markCanvasPainted(ok, () => { ok.pixels = 'ok'; });
  const boom: MockCanvas = {
    width: 10,
    height: 10,
    pixels: 'bad',
    transfers: 0,
    transferToImageBitmap() { throw new Error('transfer boom'); },
  };
  markCanvasPainted(boom, () => { boom.pixels = 'bad'; });
  let okClosed = false;
  const origTransfer = ok.transferToImageBitmap.bind(ok);
  ok.transferToImageBitmap = () => {
    const bmp = origTransfer();
    bmp.close = () => { okClosed = true; bmp.closed = true; };
    return bmp;
  };
  await assert.rejects(cropSurfaces<MockBmp>([ok, boom], fallback), /transfer boom/);
  assert.equal(okClosed, true, 'already-created sibling bitmaps are closed if a synchronous transfer throws');
});
