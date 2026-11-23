import assert from 'node:assert/strict';
import { test } from 'node:test';
import { encode } from '@msgpack/msgpack';
import { Lane, type LaneHost, type LaneView, type Painted, type Presented } from './lane.ts';
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

function rig(views: Record<string, LaneView> = {}, extra: { budget?: number; crop?: LaneHost<Fake>['crop']; clock?: () => number } = {}) {
  const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => { posted.push(m); if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target); } });
  const posted: PageMessage[] = [], released: number[] = [], failures: string[] = [], later: (() => void)[] = [], hits: (string | null)[] = [];
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
    later: run => later.push(run),
    hit: (_widget, key) => hits.push(key),
  });
  const mount = (widget: number, subs: { id: number; channel: 'panel' | 'followed'; product?: string }[], view = 'fields', surfaces = 0, timing: Timing = {}) => {
    for (const sub of subs) lane.follow(sub.id, sub.channel, sub.product);
    lane.mount(widget, subs.map(sub => sub.id), view);
    const list: (string | null)[] = [];
    shown.set(widget, list);
    page.mount(widget, { route: ROUTE, lane: 0, offset: timing.offset ?? 0, pin: timing.pin ?? null,
      surfaces: Array.from({ length: surfaces }, (_, k) => ({ install: (b: Fake) => { list[k] = b.content; }, clear: () => { list[k] = null; } })) });
    lane.bind(widget, { generation: 1, surfaces: Array.from({ length: surfaces }, () => ({ width: 1, height: 1, dpr: 1 })), offset: timing.offset ?? 0, pin: timing.pin ?? null,
      ...timing.dated ? { dated: timing.dated } : {} });
  };
  const step = async (T: number, options: Parameters<Page<Fake>['publish']>[1] = {}) => { page.publish(T, options); await flush(); await flush(); page.frame(); await flush(); };
  return { page, lane, posted, released, failures, later, hits, presented, shown, mount, step, frames: () => posted.filter((m): m is FrameRequest => m.type === 'frame') };
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
    lane.receive('panels', panels([{ id: 0, hour, kind: 'completed' }]));
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

test('review 5 finding 3: an entry whose crop failed is retired and shown as a named failure, never left in flight', async () => {
  const view: LaneView = { paint: (): Painted => ({ surfaces: [{ source: { width: 1, height: 1, content: 'p' } as never, content: 'p' }], fields: {}, motion: false }) };
  const { page, lane, mount, step, failures, presented } = rig({ view }, { crop: async () => { throw new Error('canvas lost'); } });
  mount(1, [{ id: 3, channel: 'panel' }], 'view', 1);
  lane.bindRoute(3, ROUTE, NATIVE);
  lane.receive('panels', panels([{ id: 3, hour: 10, kind: 'data', stepped: true, data: {} }, { id: 0, hour: 20, kind: 'completed' }]));
  await step(12, { playing: false });
  assert.deepEqual(failures, [], 'the lane keeps serving');
  assert.deepEqual([page.view(1).mode, page.view(1).reason], ['failed', 'drawing failed: canvas lost']);
  assert.deepEqual(presented.flatMap(p => p.entries.map(e => e.mode)), ['failed']);
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
  lane.receive('follow', message(10, [{ id: 5, answer: rows([1n, 2n], [1n, 2n]) }], false, NATIVE));
  const before = total();
  const changed = message(20, [{ id: 5, answer: rows([9n], [3n], { changed: true }) }], false, NATIVE);
  lane.receive('follow', changed);
  const record = changed.subarray(24);
  const charge = 16 + 2 * Math.max(new DataView(record.buffer, record.byteOffset).getUint32(8, true), new DataView(record.buffer, record.byteOffset).getUint32(12, true));
  assert.equal(total() - before, charge, 'the refused record is released, never left charged');
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
  const bad = message(10, [{ id: 5, answer: rows([1n], [2n]) }], false, NATIVE);
  bad[24 + 16] = 9;
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

test('a protocol fault on one subscription fails only widgets using it while sibling widgets on the same lane keep serving', async () => {
  const { page, lane, mount, step, failures } = rig();
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
  assert.equal(page.view(1).mode, 'failed');
  assert.match(page.view(1).reason ?? '', /does not hold/);
  assert.deepEqual([page.view(2).mode, valueOf(page.view(2), 7)], ['picture', 75n]);
  lane.receive('follow', message(30, [{ id: 5, answer: rows([1n, 2n], [30n, 40n]) }], true, NATIVE));
  await step(30, { playing: false });
  assert.deepEqual([page.view(1).mode, valueOf(page.view(1), 5)], ['picture', 30n], 'a whole reset table clears the isolated subscription fault');
});

test('quiet frames visit zero unchanged widgets in serve and prune across 114 widgets, and K changed visits O(K)', async () => {
  const { page, lane, mount, step } = rig({}, { clock: () => 0 });
  const initial: { id: number; answer: ReturnType<typeof rows> }[] = [];
  for (let i = 1; i <= 114; i++) {
    const sub = i * 2 + 1;
    mount(i, [{ id: sub, channel: 'followed' }]);
    lane.bindRoute(sub, ROUTE, NATIVE);
    initial.push({ id: sub, answer: rows([1n], [BigInt(i)]) });
  }
  lane.receive('follow', message(10, initial, false, NATIVE));
  await step(10, { playing: true, step: 24, speed: 1440 });
  await step(34, { playing: true, step: 24, speed: 1440 });
  const beforeVisited = lane.stats.visited;
  const beforePruned = lane.stats.pruned;
  for (let h = 58; h <= 200; h += 24) {
    await step(h, { playing: true, step: 24, speed: 1440 });
  }
  assert.equal(lane.stats.visited - beforeVisited, 0, `quiet frames visited ${lane.stats.visited - beforeVisited} widgets`);
  assert.equal(lane.stats.pruned - beforePruned, 0, `quiet frames pruned ${lane.stats.pruned - beforePruned} subs`);
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
