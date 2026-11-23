import assert from 'node:assert/strict';
import { test } from 'node:test';
import { encode } from '@msgpack/msgpack';
import { Lane, type LaneHost, type LaneView, type Painted, type Presented } from './lane.ts';
import { laneDispatch } from './laneDispatch.ts';
import type { FromLane, FromPull, ToLane } from './laneMessages';
import { fieldsView } from './views/fields.ts';
import { Page, type Bitmap, type FrameRequest, type PageMessage } from '../dashboard/follow/page.ts';
import { message } from '../dashboard/follow/messages.fixture.ts';
import { parseAnswer, cellValue } from '../dashboard/follow/message.ts';
import { splitForLanes } from '../dashboard/follow/split.ts';
import { componentReading } from '../dashboard/follow/useFollowed.ts';
import { installedEntry } from '../dashboard/follow/store.ts';
import type { Pipeline } from '../dashboard/follow/pipeline';

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
  const { page, lane, mount, step, hits } = rig({ view });
  mount(1, [{ id: 3, channel: 'panel' }], 'view');
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }]));
  await step(10, { playing: false });
  lane.receive('panels', panels([{ id: 3, hour: 20, kind: 'data', stepped: true, data: {} }]));
  page.publish(20, { playing: false });
  await flush(); await flush();
  lane.pointer(1, 0, [1, 1]);
  await flush(); await flush();
  assert.deepEqual(hits, ['row:10']);
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

test('deferred frame whose seek was superseded by stream before run never paints or prunes the new generation, still applies acks and emits served', async () => {
  const scheduled: (() => void)[] = [];
  const presented: Presented<Fake>[] = [];
  const served: { seek: number; target: number; exact: boolean; changed: number; ms: number }[] = [];
  const released: number[] = [];
  const lane = new Lane<Fake>({
    lane: 0,
    lanes: 1,
    views: () => fieldsView(() => undefined),
    crop: async () => [],
    schedule: run => { scheduled.push(run); },
    present: presentation => { presented.push(presentation); },
    release: bytes => { released.push(bytes); },
    fail: reason => { throw new Error(`unexpected lane failure: ${reason}`); },
    later: () => {},
    served: frame => { served.push(frame); },
  });
  lane.follow(5, 'followed');
  lane.mount(1, [5], 'fields');
  lane.bind(1, { generation: 1, surfaces: [], offset: 0, pin: null });
  lane.bindRoute(5, ROUTE, NATIVE, 0);

  lane.receive('follow', message(1000, [{ id: 5, hour: 990, answer: rows([1n], [990n]) }], false, NATIVE, 0));
  lane.frame({ type: 'frame', serial: 1, now: 990, target: 990, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
  while (scheduled.length) { scheduled.shift()!(); await flush(); }
  assert.equal(presented.length, 1);
  const firstEntryId = presented[0]!.entries[0]!.id;

  lane.frame({
    type: 'frame', serial: 2, now: 1000, target: 1024, exact: false, seek: 0, speed: 1440,
    installed: [{ widget: 1, binding: 1, id: firstEntryId }], refused: [], inspect: [],
  });
  assert.equal(scheduled.length, 1, 'stale seek-0 playing frame is deferred');

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

  lane.frame({ type: 'frame', serial: 3, now: 10, target: 10, exact: true, seek: 1, speed: 0, installed: [], refused: [], inspect: [] });
  while (scheduled.length) { scheduled.shift()!(); await flush(); }
  assert.equal(presented.length, presentedBefore + 1, 'following exact seek-1 request for hour 10 paints hour 10');
  const seek1Entry = presented.at(-1)!.entries[0]!;
  assert.deepEqual([seek1Entry.gen.seek, seek1Entry.h], [1, 10]);
  const answer = parseAnswer(seek1Entry.fields['answer:5'] as Uint8Array);
  assert.ok(answer.kind === 'rows' && cellValue(answer, 0) === 10n);
  lane.frame({
    type: 'frame', serial: 4, now: 10, target: 10, exact: true, seek: 1, speed: 0,
    installed: [{ widget: 1, binding: 1, id: seek1Entry.id }], refused: [], inspect: [],
  });
  while (scheduled.length) { scheduled.shift()!(); await flush(); }
  assert.ok(released.reduce((sum, b) => sum + b, 0) > releasedBefore, 'acknowledged seek-0 fact is released once replaced');
});

test('laneDispatch with macrotask scheduling keeps slow paints within one paint of the newest request, while microtask scheduling falls linearly behind', async () => {
  const simulate = async (deferToMailbox: boolean) => {
    const frameMs = 16;
    const paintMs = 48;
    const stepHours = 24;
    const ticks = 80;
    let clockMs = 0;
    let busyUntil = 0;
    let serial = 0;
    let newestTarget = 0;
    let paints = 0;
    const mailbox: (() => void)[] = [];
    const paintedLags: number[] = [];
    const port = { onmessage: null as ((event: MessageEvent<FromPull>) => void) | null, postMessage() {} } as unknown as MessagePort;

    const view: LaneView = {
      paint: (input): Painted => {
        paints++;
        clockMs += paintMs;
        busyUntil = clockMs;
        const h = input.pictures[0]!.h;
        return { surfaces: [], fields: { h: { value: h, content: `h:${h}` } }, motion: false };
      },
    };

    const dispatch = laneDispatch((msg: FromLane) => {
      if (msg.type === 'served' && msg.changed > 0) {
        paintedLags.push(newestTarget - msg.target);
      }
    }, {
      views: () => view,
      crop: async () => [],
      later: () => {},
      ...deferToMailbox ? { schedule: run => { mailbox.push(run); } } : {},
    });

    const post = (m: ToLane) => { mailbox.push(() => dispatch(m)); };
    const drainMicrotasks = async () => {
      for (let i = 0; i < 6; i++) await Promise.resolve();
    };

    post({ type: 'init', lane: 0, lanes: 1, port, rate: 1 });
    post({ type: 'follow', sub: 5, channel: 'followed' });
    post({ type: 'mount', widget: 1, subs: [5], view: 'slow' });
    post({ type: 'bind', widget: 1, binding: { generation: 1, surfaces: [], offset: 0, pin: null } });
    post({ type: 'route', sub: 5, route: ROUTE, native: NATIVE, seek: 0, offset: 0 });
    while (mailbox.length) { mailbox.shift()!(); await drainMicrotasks(); }

    let pushed = -stepHours;
    const pushFacts = (limit: number) => {
      while (pushed + stepHours <= limit) {
        pushed += stepHours;
        const h = pushed;
        const pkt = message(h + stepHours - 1, [{ id: 5, hour: h, answer: rows([1n], [BigInt(h)]) }], false, NATIVE);
        mailbox.push(() => { port.onmessage?.({ data: { kind: 'follow', bytes: pkt } } as MessageEvent<FromPull>); });
      }
    };

    for (let tick = 1; tick <= ticks; tick++) {
      const tickMs = tick * frameMs;
      const nowHour = tick * stepHours;
      newestTarget = nowHour + stepHours;
      pushFacts(newestTarget);
      post({
        type: 'frame', serial: ++serial, now: nowHour, target: newestTarget, exact: false,
        seek: 0, speed: 1440, installed: [], refused: [], inspect: [],
      });
      if (tickMs >= busyUntil) {
        clockMs = tickMs;
        while (mailbox.length && clockMs <= tickMs) {
          mailbox.shift()!();
          await drainMicrotasks();
        }
      }
    }

    const paintBudgetHours = Math.ceil((paintMs / frameMs) * stepHours);
    return { paints, maxLag: Math.max(...paintedLags), endLag: paintedLags.at(-1)!, paintBudgetHours };
  };

  const macro = await simulate(true);
  assert.ok(macro.maxLag <= macro.paintBudgetHours,
    `macrotask schedule must keep painted target within one paint (${macro.paintBudgetHours}h) of newest request, got max=${macro.maxLag}h`);
  assert.ok(macro.paints < 40, `macrotask schedule must drop intermediate frames, painted ${macro.paints} of 80 ticks`);

  const micro = await simulate(false);
  assert.ok(micro.endLag > 1000,
    `regression proof: microtask schedule must fall linearly behind (> 1000h), got endLag=${micro.endLag}h`);
});
