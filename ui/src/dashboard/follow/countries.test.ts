import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Lane } from '../../draw/lane.ts';
import { fieldsView } from '../../draw/views/fields.ts';
import { Page, type Bitmap, type Entry, type PageMessage } from './page.ts';
import { parseAnswer, cellValue } from './message.ts';
import { answerBody, message } from './messages.fixture.ts';
import { charge, splitForLanes } from './split.ts';

interface Fake extends Bitmap { closed: boolean }
const NATIVE = 3, ROUTE = 1, LANES = 2, WIDGETS = 12, SUBS = 4, COUNTRIES = 1500, FRAMES = 60, STEP = 24;
const flush = async () => { for (let i = 0; i < 3; i++) await new Promise<void>(resolve => setImmediate(resolve)); };
const valueAt = (sub: number, row: number, day: number) => BigInt((row * 7919 + day * 31 + sub) % 1_000_003);

test('a recording of 1,500 countries plays through the lanes and the page: every widget current, every value right, retained bytes bounded', async () => {
  const lanes: Lane<Fake>[] = [], failures: string[] = [];
  let charged = 0, released = 0;
  const page = new Page<Fake>({ lanes: LANES, post: (lane, m: PageMessage) => {
    for (const [index, target] of lanes.entries()) if (lane === 'all' || lane === index) {
      const copy = structuredClone(m);
      if (copy.type === 'frame') target.frame(copy); else if (copy.type === 'stream') target.stream(copy.route, copy.seek, copy.target);
    }
  } });
  const cloned = (entry: Entry<Fake>): Entry<Fake> => ({ ...structuredClone({ ...entry, surfaces: [] }), surfaces: entry.surfaces });
  for (let index = 0; index < LANES; index++) lanes.push(new Lane<Fake>({
    lane: index, lanes: LANES, views: () => fieldsView(() => undefined),
    crop: async sources => sources.map((): Fake => ({ closed: false, width: 1, close() { this.closed = true; } })),
    present: presentation => page.offer({ ...presentation, entries: presentation.entries.map(cloned) }),
    release: bytes => { released += bytes; }, fail: reason => failures.push(reason), later: () => {},
  }));
  const subs = Array.from({ length: SUBS }, (_, at) => (at + 1) * LANES + at % LANES);
  for (const sub of subs) { lanes[sub % LANES]!.follow(sub, 'followed'); lanes[sub % LANES]!.bindRoute(sub, ROUTE, NATIVE); }
  const keys = Array.from({ length: COUNTRIES }, (_, row) => BigInt(row + 1));
  for (let widget = 0; widget < WIDGETS; widget++) {
    const sub = subs[widget % SUBS]!;
    lanes[sub % LANES]!.mount(widget, [sub], 'fields');
    page.mount(widget, { route: ROUTE, lane: sub % LANES, surfaces: [] });
  }
  page.publish(0, { playing: true, step: STEP, speed: 1440, seek: 0 });
  let largest = 0;
  const names = keys.map((_, row) => `C${row}`);
  const whole = answerBody({ keys, columns: 1, values: keys, names }).length;
  const shown = new Map(subs.map(sub => [sub, keys.map(() => 0n)]));
  for (let day = 0; day < FRAMES; day++) {
    const changed = day === 0 ? keys.map((_, row) => row) : Array.from({ length: COUNTRIES / 20 }, (_, k) => (day * 7 + k * 20) % COUNTRIES).sort((a, b) => a - b);
    const sent = message(day * STEP, subs.map(id => ({ id, answer: { keys: changed.map(row => keys[row]!), columns: 1, values: changed.map(row => valueAt(id, row, day)),
      names: day === 0 ? names : undefined, changed: day > 0 }, held: day > 0 ? whole : 0 })), false, NATIVE);
    for (const sub of subs) for (const row of changed) shown.get(sub)![row] = valueAt(sub, row, day);
    charged += charge(sent);
    const split = splitForLanes(sent, LANES);
    assert.ok(split.kind === 'follow');
    released += split.header;
    split.lanes.forEach((part, lane) => { if (part) lanes[lane]!.receive('follow', structuredClone(part)); });
    await flush();
    page.publish(day * STEP + STEP - 1, { playing: true, step: STEP, speed: 1440 });
    await flush();
    page.frame();
    await flush();
    for (let widget = 0; widget < WIDGETS; widget++) {
      const view = page.view(widget), sub = subs[widget % SUBS]!;
      assert.equal(view.h, day * STEP, `widget ${widget} shows day ${day}`);
      const answer = parseAnswer(view.fields?.[`answer:${sub}`] as Uint8Array);
      assert.ok(answer.kind === 'rows' && answer.rows === COUNTRIES);
      const expected = shown.get(sub)!;
      for (let row = 0; row < COUNTRIES; row++) if (cellValue(answer, row) !== expected[row]) assert.fail(`widget ${widget} row ${row} on day ${day}: ${cellValue(answer, row)}, last changed to ${expected[row]}`);
    }
    largest = Math.max(largest, charged - released);
  }
  assert.deepEqual(failures, []);
  const table = charge(message(0, [{ id: subs[0]!, answer: { keys, columns: 1, values: keys, names } }], false, NATIVE));
  assert.ok(largest <= SUBS * table * 3, `${largest} bytes retained at most for ${SUBS} readings of ${COUNTRIES} countries; a whole table charges ${table}`);
});

test('1,500 countries with 114 mounted widgets does zero work on quiet frames and O(K) work when K widgets change', async () => {
  const lanes: Lane<Fake>[] = [], failures: string[] = [];
  const page = new Page<Fake>({ lanes: LANES, post: (lane, m: PageMessage) => {
    for (const [index, target] of lanes.entries()) if (lane === 'all' || lane === index) {
      if (m.type === 'frame') target.frame(m); else if (m.type === 'stream') target.stream(m.route, m.seek, m.target);
    }
  } });
  for (let index = 0; index < LANES; index++) lanes.push(new Lane<Fake>({
    lane: index, lanes: LANES, views: () => fieldsView(() => undefined),
    crop: async sources => sources.map((): Fake => ({ closed: false, width: 1, close() { this.closed = true; } })),
    present: presentation => page.offer(presentation),
    release: () => {}, fail: reason => failures.push(reason), later: () => {}, clock: () => 0,
  }));
  const keys = Array.from({ length: COUNTRIES }, (_, row) => BigInt(row + 1));
  const names = keys.map((_, row) => `C${row}`);
  const MOUNTED = 114;
  const subs = Array.from({ length: MOUNTED }, (_, i) => (i + 1) * LANES + (i % LANES));
  for (let i = 0; i < MOUNTED; i++) {
    const sub = subs[i]!, lane = sub % LANES;
    lanes[lane]!.follow(sub, 'followed');
    lanes[lane]!.bindRoute(sub, ROUTE, NATIVE);
    lanes[lane]!.mount(i + 1, [sub], 'fields');
    page.mount(i + 1, { route: ROUTE, lane, surfaces: [] });
  }
  const whole = answerBody({ keys, columns: 1, values: keys, names }).length;
  for (let i = 0; i < MOUNTED; i++) {
    const sub = subs[i]!;
    const sent = message(0, [{ id: sub, answer: { keys, columns: 1, values: keys, names } }], false, NATIVE);
    const split = splitForLanes(sent, LANES);
    assert.ok(split.kind === 'follow');
    split.lanes.forEach((part, lane) => { if (part) lanes[lane]!.receive('follow', part); });
  }
  page.publish(0, { playing: true, step: STEP, speed: 1440, seek: 0 });
  await flush(); page.frame(); await flush();
  const warmPatch = message(STEP, subs.map(id => ({ id, answer: { keys: [keys[0]!], columns: 1, values: [BigInt(id + 100)], changed: true }, held: whole })), false, NATIVE);
  const warmSplit = splitForLanes(warmPatch, LANES);
  assert.ok(warmSplit.kind === 'follow');
  warmSplit.lanes.forEach((part, lane) => { if (part) lanes[lane]!.receive('follow', part); });
  page.publish(STEP, { playing: true, step: STEP, speed: 1440 });
  await flush(); page.frame(); await flush();
  page.publish(2 * STEP, { playing: true, step: STEP, speed: 1440 });
  await flush(); page.frame(); await flush();
  const pageBefore = page.stats.visited;
  const laneBefore = lanes.reduce((sum, l) => sum + l.stats.visited, 0);
  const pruneBefore = lanes.reduce((sum, l) => sum + l.stats.pruned, 0);
  for (let f = 3; f <= 12; f++) {
    page.publish(f * STEP, { playing: true, step: STEP, speed: 1440 });
    await flush();
  }
  assert.equal(page.stats.visited - pageBefore, 0, 'page visits zero widgets on quiet frames');
  assert.equal(lanes.reduce((sum, l) => sum + l.stats.visited, 0) - laneBefore, 0, 'lanes visit zero widgets on quiet frames');
  assert.equal(lanes.reduce((sum, l) => sum + l.stats.pruned, 0) - pruneBefore, 0, 'lanes prune zero subs on quiet frames');
  const laneBeforeK = lanes.reduce((sum, l) => sum + l.stats.visited, 0);
  const patchHour = 13 * STEP;
  const patched = message(patchHour, [
    { id: subs[0]!, answer: { keys: [keys[0]!], columns: 1, values: [999_999n], changed: true }, held: whole },
    { id: subs[1]!, answer: { keys: [keys[1499]!], columns: 1, values: [888_888n], changed: true }, held: whole },
  ], false, NATIVE);
  const split = splitForLanes(patched, LANES);
  assert.ok(split.kind === 'follow');
  split.lanes.forEach((part, lane) => { if (part) lanes[lane]!.receive('follow', part); });
  page.publish(patchHour, { playing: true, step: STEP, speed: 1440 });
  await flush(); page.frame(); await flush();
  assert.ok(lanes.reduce((sum, l) => sum + l.stats.visited, 0) - laneBeforeK <= 4, 'only the 2 changed widgets are visited across lanes');
  const a0 = parseAnswer(page.view(1).fields?.[`answer:${subs[0]}`] as Uint8Array);
  const a1 = parseAnswer(page.view(2).fields?.[`answer:${subs[1]}`] as Uint8Array);
  assert.ok(a0.kind === 'rows' && cellValue(a0, 0) === 999_999n && cellValue(a0, 1) === 2n);
  assert.ok(a1.kind === 'rows' && cellValue(a1, 1499) === 888_888n && cellValue(a1, 1) === 2n);
  assert.deepEqual(failures, []);
});
