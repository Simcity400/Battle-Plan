import assert from 'node:assert/strict';
import { test } from 'node:test';
import { encode } from '@msgpack/msgpack';
import { Lane, type LaneView, type Painted, type Presented } from './lane.ts';
import { fieldsView } from './views/fields.ts';
import { Page, type Bitmap, type FrameRequest, type PageMessage } from '../dashboard/follow/page.ts';
import { message } from '../dashboard/follow/messages.fixture.ts';
import { parseAnswer, cellValue } from '../dashboard/follow/message.ts';
import { splitForLanes } from '../dashboard/follow/split.ts';

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

function rig(views: Record<string, LaneView> = {}, extra: { budget?: number } = {}) {
  const page = new Page<Fake>({ lanes: 1, post: (_lane, m: PageMessage) => { posted.push(m); if (m.type === 'frame') lane.frame(m); else if (m.type === 'stream') lane.stream(m.route, m.seek, m.target); } });
  const posted: PageMessage[] = [], released: number[] = [], failures: string[] = [], later: (() => void)[] = [], hits: (string | null)[] = [];
  const presented: Presented<Fake>[] = [], charged: number[] = [];
  const shown = new Map<number, (string | null)[]>();
  const lane: Lane<Fake> = new Lane<Fake>({
    lane: 0, lanes: 1, ...extra,
    views: kind => views[kind] ?? fieldsView(() => (panel => (panel.data as { shape?: string } | undefined)?.shape)),
    crop: async sources => sources.map((source): Fake => ({ content: (source as unknown as { content: string }).content, closed: false, width: 1, close() { this.closed = true; } })),
    present: presentation => { presented.push(presentation); page.offer(presentation); },
    release: bytes => released.push(bytes),
    charge: bytes => charged.push(bytes),
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
  return { page, lane, posted, released, charged, failures, later, hits, presented, shown, mount, step, frames: () => posted.filter((m): m is FrameRequest => m.type === 'frame') };
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
  lane.receive('panels', panels([{ id: 3, hour: 16, kind: 'data', stepped: true, data: { shape: 'A', n: 2 } }]));
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

test('addendum 4: answers of another ranked window never install', async () => {
  const { page, lane, mount, step } = rig();
  mount(1, [{ id: 5, channel: 'followed' }]);
  lane.bindRoute(5, ROUTE, NATIVE);
  lane.rank(5, 2);
  page.rebind(1, { rank: 2 });
  lane.bind(1, { generation: 2, surfaces: [], offset: 0, pin: null });
  lane.receive('follow', message(10, [{ id: 5, answer: { keys: [1n], columns: 2, values: [5n, 0n], window: 1 } }], false, NATIVE));
  await step(10, { playing: false });
  assert.equal(page.view(1).mode, 'reading');
  lane.receive('follow', message(11, [{ id: 5, answer: { keys: [1n], columns: 2, values: [6n, 0n], window: 2 } }], false, NATIVE));
  await step(11);
  assert.equal(page.view(1).h, 11);
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
    present: presentation => page.offer(presentation), release: () => {}, charge: () => {}, fail: () => {}, later: () => {} }));
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
  assert.deepEqual([failures, page.view(1).mode], [[], 'picture'], 'the old stream\'s change is dropped, not merged onto a not-recorded answer');
  await step(9, { playing: false, seek: true });
  assert.deepEqual([page.view(1).h, valueOf(page.view(1), 5, 0), valueOf(page.view(1), 5, 1)], [9, 80n, 91n], 'the early stream merges onto its own table once its seek is current');
});
