import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, cellState, cellValue, parseFollowed, rowHour, rowKeys, rowName, rowReason } from './message.ts';
import type { FollowedAnswer } from './message.ts';
import { datedPanel, fakeTransport, manualFrames, message } from './messages.fixture.ts';
import { BarCell, TextCell, formatAmountFast, paintArmy, paintMeasure, paintStability } from './painters.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { FollowStore } from './store.ts';
import { Followed } from '../../draw/followed.ts';
import { StructureQueue } from './structureQueue.ts';
import { liveKey } from '../../charts/live.ts';
import { DatedStore } from './datedStore.ts';
import { FollowDecoder } from './decode.ts';
import type { FollowEntry } from './store.ts';

const one = (key: bigint, value: bigint) => ({ keys: [key], columns: 1, values: [value] });

test('queued row structures publish only at their hour and a reset removes abandoned rows through the next input', () => {
  const q = new StructureQueue(), published: number[] = [];
  const record = (hour: number, seek = 5) => ({ id: 1, seek, hour, through: hour });
  const drain = (hour: number) => q.drain(hour, r => published.push(r.hour));
  q.add(record(12)); q.add(record(30)); q.add(record(20));
  drain(11); assert.deepEqual(published, []);
  drain(15); assert.deepEqual(published, [12]);
  q.discard(new Set([1]), 20);
  q.add(record(24)); drain(40);
  assert.deepEqual(published, [12, 24]);
  q.add(record(48)); q.discard(new Set([1])); q.add(record(3, 6)); drain(50);
  assert.deepEqual(published, [12, 24, 3]);
  q.add(record(60)); q.add({ ...record(68), id: 2 }); q.add(record(72));
  q.add({ ...record(72), through: 75 });
  const due: number[][] = [];
  q.drain(75, r => due.push([r.id, r.hour, r.through]));
  assert.deepEqual(due.sort((a, b) => a[0]! - b[0]!), [[1, 72, 75], [2, 68, 68]],
    'a delayed page frame publishes only the newest eligible structure per subscription');
});

function cell() {
  const node = { data: '' }, host = { title: '' };
  return { cell: new TextCell(node, host), node, host };
}

test('two widgets share one follow and the last unfollow sends unfollow', () => {
  const transport = fakeTransport();
  const store = new FollowStore(transport, manualFrames().schedule);
  const first = store.acquire({ table: 2, columns: [3], keys: ['100'] });
  const second = store.acquire({ keys: ['100'], columns: [3], table: 2 });
  assert.equal(first.entry, second.entry, 'one entry for one query however its keys are ordered');
  assert.deepEqual(transport.sent, ['follow 1 {"table":2,"columns":[3],"keys":["100"]}']);
  first.release();
  first.release();
  assert.equal(transport.sent.length, 1, 'a holder remains');
  second.release();
  assert.deepEqual(transport.sent.slice(1), ['unfollow 1']);
  const again = store.acquire({ table: 2, columns: [3], keys: ['100'] });
  assert.equal(again.entry.id, 2, 'a new follow after the last one ended');
});

test('a painter runs at most once a frame and only when its version moved', () => {
  const transport = fakeTransport();
  const frames = manualFrames();
  const store = new FollowStore(transport, frames.schedule);
  const handle = store.acquire({ q: 1 });
  const painted: number[] = [];
  store.painter(handle.entry, entry => painted.push(entry.hour));
  assert.equal(frames.run(), 0);
  assert.deepEqual(painted, []);
  transport.deliver(message(24, [{ id: 1, answer: one(100n, 5n) }]));
  transport.deliver(message(48, [{ id: 1, answer: one(100n, 6n) }]));
  transport.deliver(message(72, [{ id: 1, answer: one(100n, 7n) }]));
  assert.equal(frames.pending, 1, 'three messages, one frame');
  frames.run();
  assert.deepEqual(painted, [72], 'painted once, with the newest');
  frames.run();
  transport.deliver(message(96, [{ id: 2, answer: one(1n, 1n) }], false, 1));
  assert.equal(frames.pending, 0, 'nothing due when its entry did not move');
  assert.deepEqual(painted, [72]);
});

test('an off-screen widget is not painted and catches up when shown', () => {
  const transport = fakeTransport();
  const frames = manualFrames();
  const store = new FollowStore(transport, frames.schedule);
  const handle = store.acquire({ q: 1 });
  const shown: bigint[] = [], hidden: bigint[] = [];
  const value = (into: bigint[]) => (entry: { answer: FollowedAnswer | undefined }) => {
    if (entry.answer?.kind === 'rows') into.push(cellValue(entry.answer, 0));
  };
  store.painter(handle.entry, value(shown));
  const off = store.painter(handle.entry, value(hidden), false);
  for (let hour = 1; hour <= 3; hour++) {
    transport.deliver(message(hour * 24, [{ id: 1, answer: one(100n, BigInt(hour)) }]));
    frames.run();
  }
  assert.deepEqual(shown, [1n, 2n, 3n]);
  assert.deepEqual(hidden, [], 'never painted off screen');
  off.setVisible(true);
  assert.equal(frames.pending, 1);
  frames.run();
  assert.deepEqual(hidden, [3n], 'caught up with the newest at the next frame');
  off.setVisible(false);
  off.setVisible(true);
  assert.equal(frames.pending, 0, 'already current');
});

test('one message lands in every entry it names, parsed once, and 500 painters take one frame', () => {
  const transport = fakeTransport();
  const frames = manualFrames();
  const store = new FollowStore(transport, frames.schedule);
  const slots: ReturnType<typeof cell>[] = [];
  for (let id = 1; id <= 500; id++) {
    const handle = store.acquire({ q: id });
    const slot = cell();
    slots.push(slot);
    store.painter(handle.entry, entry => paintMeasure(entry.answer, [slot.cell], 5, 0));
  }
  frames.run();
  const bytes = message(100, Array.from({ length: 500 }, (_, at) => ({ id: at + 1, answer: one(BigInt(at), BigInt(at) * 100_000n) })));
  assert.ok(bytes.length < 64 * 1024, `${bytes.length} bytes`);
  transport.deliver(bytes);
  assert.equal(frames.pending, 1, 'one frame for every entry');
  for (let round = 0; round < 3; round++) {
    transport.deliver(message(100 + round, Array.from({ length: 500 }, (_, at) => ({ id: at + 1, answer: one(BigInt(at), BigInt(at + round) * 100_000n) }))));
    assert.equal(frames.run(), 1);
  }
  assert.equal(store.counts.paints, 500 * 3, 'each painter runs once per frame with a reading');
  assert.equal(slots[499]!.node.data, String(499 + 2));
});

test('a message reads back as written, numbers as typed arrays over its bytes', () => {
  const bytes = message(1234, [
    { id: 7, hour: 1200, answer: { keys: [3n, (1n << 70n) + 5n], columns: 2, values: [1n, -2n, 0n, 9n], states: [CELL.value, CELL.value, CELL.missing, CELL.value], reasons: [null, 'GER is held by no country'] } },
    { id: 8, answer: { notRecorded: 'Countries was not recorded at this hour' } },
    { id: 9, answer: { keys: [11n], columns: 1, values: [4n], hours: [1100] } },
  ], true);
  const parsed = parseFollowed(bytes);
  assert.equal(parsed.hour, 1234);
  assert.equal(parsed.reset, true);
  const [rows, notRecorded, log] = parsed.answers;
  assert.equal(rows!.hour, 1200);
  assert.ok(rows!.answer.kind === 'rows');
  assert.equal(rows!.answer.views.u8.buffer, bytes.buffer, 'no copies');
  assert.equal(rows!.answer.views, parsed.answers[2]!.answer.kind === 'log' ? parsed.answers[2]!.answer.views : null, 'one set of views a message');
  assert.deepEqual([0, 1, 2, 3].map(at => cellValue(rows!.answer as never, at)), [1n, -2n, 0n, 9n]);
  assert.equal(rowKeys(rows!.answer), `3,${(1n << 70n) + 5n}`);
  assert.deepEqual(notRecorded!.answer, { kind: 'notRecorded', reason: 'Countries was not recorded at this hour' });
  assert.ok(log!.answer.kind === 'log');
  assert.equal(rowHour(log!.answer, 0), 1100);
  const shifted = new Uint8Array(bytes.length + 3);
  shifted.set(bytes, 3);
  const moved = parseFollowed(shifted.subarray(3)).answers[0]!.answer;
  assert.ok(moved.kind === 'rows');
  assert.deepEqual([0, 1, 2, 3].map(at => cellValue(moved, at)), [1n, -2n, 0n, 9n], 'a view off an eight-byte boundary is read too');
  assert.equal(rowKeys(moved), rowKeys(rows!.answer));
  assert.throws(() => parseFollowed(bytes.subarray(0, bytes.length - 8)), /ends early|past/);
});

test('the measure and stability painters write values, and a missing value as its reason, never zero', () => {
  const slots = () => [cell(), cell(), cell()];
  const answer = parseFollowed(message(10, [{ id: 1, answer: {
    keys: [1n, 2n, 0n], columns: 1, values: [123_456_789n, 0n, 0n],
    states: [CELL.value, CELL.unrecorded, CELL.noRow], reasons: [null, "SOV's politicalPower was not recorded at this hour", 'ITA is held by no country'],
  } }])).answers[0]!.answer;
  const measure = slots();
  paintMeasure(answer, measure.map(slot => slot.cell), 5, 1);
  assert.deepEqual(measure.map(slot => [slot.node.data, slot.host.title]),
    [['1,234.6', ''], ['—', "SOV's politicalPower was not recorded at this hour"], ['—', 'ITA is held by no country']]);
  const stability = slots();
  paintStability(parseFollowed(message(10, [{ id: 1, answer: { keys: [1n], columns: 1, values: [61_500n] } }])).answers[0]!.answer, [stability[0]!.cell], 5, 1);
  assert.equal(stability[0]!.node.data, '61.5%');
  paintStability({ kind: 'notRecorded', reason: 'CountryStability was not recorded at this hour' }, stability.map(slot => slot.cell), 5, 0);
  assert.deepEqual(stability.map(slot => slot.node.data), ['—', '—', '—']);
  assert.equal(stability[2]!.host.title, 'CountryStability was not recorded at this hour');
  let writes = 0;
  const counted = { get data() { return ''; }, set data(_: string) { writes++; } };
  const unchanged = [new TextCell(counted, { title: '' })];
  paintMeasure(answer, unchanged, 5, 1);
  paintMeasure(answer, unchanged, 5, 1);
  assert.equal(writes, 1, 'an unchanged text is not written again');
});

test('the army painter draws each row as a share of the greatest', () => {
  const rows = Array.from({ length: 3 }, () => { const style = { width: '' }; const value = cell(); return { style, value, row: { bar: new BarCell(style), value: value.cell } }; });
  const answer = parseFollowed(message(10, [{ id: 1, answer: { keys: [4n, 9n, 2n], columns: 1, values: [200n, 50n, 0n], states: [CELL.value, CELL.value, CELL.unrecorded], reasons: [null, null, 'Not recorded'] } }])).answers[0]!.answer;
  paintArmy(answer, rows.map(row => row.row), 0, 0);
  assert.deepEqual(rows.map(row => [row.style.width, row.value.node.data]), [['100%', '200'], ['25%', '50'], ['0%', '—']]);
  assert.equal(rows[2]!.value.host.title, 'Not recorded');
  paintArmy({ kind: 'notRecorded', reason: 'ArmyTotals was not recorded at this hour' }, rows.map(row => row.row), 0, 0);
  assert.deepEqual(rows.map(row => [row.style.width, row.value.node.data]), [['0%', '—'], ['0%', '—'], ['0%', '—']]);
});

test('the fast amount writes every value as formatAmount does', () => {
  const edges = [0n, 1n, 4n, 5n, 49_999n, 50_000n, 99_999n, 100_000n, 150_000n, 123_456_789n, 9_007_199_254_740_991n, 9_007_199_254_740_992n, 1n << 62n];
  let seed = 12345n;
  const raws = [...edges, ...edges.map(raw => -raw)];
  for (let i = 0; i < 2000; i++) {
    seed = (seed * 6364136223846793005n + 1442695040888963407n) & ((1n << 64n) - 1n);
    raws.push(BigInt.asIntN(64, seed) >> BigInt(i % 60));
  }
  for (const raw of raws) for (const scale of [0, 2, 5]) for (let digits = 0; digits <= 6; digits++) {
    assert.equal(formatAmountFast(raw, scale, digits), formatAmount(raw, scale, digits), `${raw} at scale ${scale}, ${digits} digits`);
  }
});

const GOLDEN = '464f4c3201000000d204000003000000000000000000000007000000b0040000c800000000000000010000000300000001000000050000006400000000000000650000000000000000000000000000000000000000000000000000000000000000000000000000004e61bc0000000000000000000000000000000000000000000001030000000000ffffffff020000000400000000000000000000000100000003000000000000000300000047455203000000534f5610000000534f5620686173206e6f2076616c756503000000495441190000004954412069732068656c64206279206e6f20636f756e747279000008000000d204000040000000000000000000000000000000000000000100000027000000436f756e747269657320776173206e6f74207265636f72646564206174207468697320686f7572000000000009000000d204000050000000000000000200000001000000020000000000000005000000000000004000000000000000feffffffffffffff09000000000000004c040000000000000000000000000000ffffffff00000000ffffffff00000000';

test('the bytes the service writes read back as it wrote them', () => {
  const bytes = new Uint8Array(GOLDEN.match(/../g)!.map(pair => parseInt(pair, 16)));
  const parsed = parseFollowed(bytes);
  assert.equal(parsed.hour, 1234);
  assert.equal(parsed.reset, true);
  assert.deepEqual(parsed.answers.map(({ id, hour }) => [id, hour]), [[7, 1200], [8, 1234], [9, 1234]]);
  const [series, unrecorded, log] = parsed.answers.map(({ answer }) => answer);
  assert.ok(series!.kind === 'rows');
  assert.deepEqual([0, 1, 2].map(row => rowName(series!, row)), ['GER', 'SOV', 'ITA']);
  assert.equal(cellValue(series!, 0), 12_345_678n);
  assert.deepEqual([0, 1, 2].map(at => cellState(series!, at)), [CELL.value, CELL.missing, CELL.noRow]);
  assert.deepEqual([0, 1, 2].map(row => rowReason(series!, row)), [null, 'SOV has no value', 'ITA is held by no country']);
  assert.deepEqual(unrecorded, { kind: 'notRecorded', reason: 'Countries was not recorded at this hour' });
  assert.ok(log!.kind === 'log');
  assert.equal(rowHour(log!, 0), 1100);
  assert.equal(rowKeys(log!), String((1n << 70n) + 5n));
  assert.deepEqual([cellValue(log!, 0), cellValue(log!, 1)], [-2n, 9n]);
});

test('seventy thousand texts are each read back by their own row', () => {
  const rows = 70_000;
  const keys = Array.from({ length: rows }, (_, row) => BigInt(row));
  const reasons = keys.map((_, row) => row % 2 === 0 ? `reason ${row}` : null), names = keys.map((_, row) => row % 2 === 0 ? null : `name ${row}`);
  const [{ answer }] = parseFollowed(message(1, [{ id: 1, answer: { keys, columns: 1, values: keys.map(() => 0n), states: keys.map(() => CELL.missing), reasons, names } }])).answers;
  assert.ok(answer!.kind === 'rows');
  for (let row = 0; row < rows; row++) {
    if (rowReason(answer!, row) !== reasons[row] || rowName(answer!, row) !== names[row]) assert.fail(`row ${row}: ${rowReason(answer!, row)}, ${rowName(answer!, row)}`);
  }
});

function dated(head: () => number) {
  const decoder = new FollowDecoder();
  const store = new DatedStore(() => {}, what => decoder.release(what));
  let next = 0;
  return {
    store,
    add() {
      const entry: FollowEntry = { id: ++next, status: 'followed', answer: undefined, hour: 0, through: 0, version: 0 };
      store.add(entry, '1');
      return entry;
    },
    deliver(bytes: Uint8Array) {
      const item = decoder.item(bytes.slice().buffer);
      if (item.kind !== 'failure') store.accept(item);
      store.drain(head());
    },
    frame: () => store.drain(head()),
  };
}
const datedStep = (through: number, answers: Parameters<typeof message>[1], epoch: number, reset = false) =>
  message(through, answers, reset, 9, epoch);

test('a dated panel reads the native MessagePack bytes', () => {
  const bytes = datedPanel(1, 7, { rows: [7] });
  assert.deepEqual([...bytes.slice(44, 52)], [0x81, 0xa4, 114, 111, 119, 115, 0x91, 7]);
  const { store, add, deliver } = dated(() => 7);
  const entry = add();
  store.bind('1', 9, 1);
  deliver(bytes);
  assert.deepEqual(entry.panel?.data, { rows: [7] });
});

test('one completed product step covers every joined product without waking idle pictures or a late seed', () => {
  const dirty: number[] = [], store = new DatedStore(entry => dirty.push(entry.id), () => {});
  const decoder = new FollowDecoder();
  const entries = [1, 2, 3].map(id => {
    const entry: FollowEntry = { id, status: 'followed', answer: undefined, hour: 0, through: 0, version: 0 };
    store.add(entry, '1');
    return entry;
  });
  store.bind('1', 9, 1);
  function land(bytes: Uint8Array) {
    const item = decoder.item(bytes.slice().buffer);
    if (item.kind !== 'failure') store.accept(item);
  }
  for (const id of [1, 2, 3]) {
    const seed = datedPanel(id, 10, { value: id });
    if (id < 3) seed[33] = 1;
    land(seed);
  }
  store.drain(10);
  dirty.length = 0;
  const completed = datedPanel(0, 34);
  completed[32] = 4;
  land(completed);
  store.drain(33);
  assert.deepEqual(entries.map(e => store.coverage(e)?.through), [10, 10, 10]);
  store.drain(34);
  assert.deepEqual(entries.map(e => store.coverage(e)?.through), [34, 34, 10]);
  assert.deepEqual(dirty, []);
  const unchanged = { id: 4, status: 'followed', hour: 0, through: 0, version: 0, answer: undefined } satisfies FollowEntry;
  store.add(unchanged, '1');
  land(datedStep(34, [{ id: 4, answer: one(1n, 7n) }], 1));
  store.drain(34);
  dirty.length = 0;
  land(datedStep(58, [], 1));
  store.drain(58);
  assert.equal(store.coverage(unchanged)?.through, 58);
  assert.deepEqual(dirty, [], 'the lane clock covers unchanged answers without scheduling their painters');
  store.seek('1', 2);
  assert.ok(entries.every(e => store.coverage(e) === null));
});

test('a pending listed log limits only its own reading', () => {
  const { store, add, deliver } = dated(() => 40);
  const log = add(), country = add();
  store.bind('1', 9, 1);
  deliver(datedStep(10, [log, country].map(e => ({ id: e.id, answer: one(1n, 7n) })), 1));
  const packet = new Uint8Array(40);
  packet.set(datedStep(40, [], 1));
  const wire = new DataView(packet.buffer);
  wire.setUint32(12, 1, true);
  wire.setUint32(24, log.id, true);
  wire.setInt32(28, 10, true);
  deliver(packet);
  assert.deepEqual([log, country].map(e => store.coverage(e)?.through), [10, 40]);
});

test('a native record covers its proven unchanged interval before the next step arrives', () => {
  let hour = 12;
  const { store, add, deliver, frame } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  const packet = datedPanel(entry.id, 10, 'held');
  packet[33] = 3;
  new DataView(packet.buffer).setInt32(40, 22, true);
  deliver(packet);
  deliver(datedPanel(entry.id, 23, 'changed'));
  assert.deepEqual(store.coverage(entry), { start: 10, through: 22 });
  assert.equal(entry.panel?.data, 'held');
  hour = 23; frame();
  assert.deepEqual(store.coverage(entry), { start: 23, through: 23 });
  const followed = new Followed(() => {});
  followed.add(1, '1'); followed.bind('1', 9, 1);
  followed.land([{ kind: 'panels', batch: { route: 9, clock: 100, seek: 1, completed: undefined,
    panels: [{ id: 1, hour: 110, through: 122, stepped: true, panel: { data: 'held', refused: null } }] } }]);
  followed.drain(12);
  assert.equal(followed.snapshot(1)?.through, 112, 'inspection stops at the presented hour on the panel clock');
});

test('page seeks admit their own packets immediately and a native reset supersedes its queued future', () => {
  let head = 10;
  const { store, add, deliver, frame } = dated(() => head);
  const entry = add(), panel = add();
  store.bind('1', 9, 17);
  const step = (at: number, seek: number, reset = false) => deliver(datedStep(at, [{ id: entry.id, answer: one(1n, BigInt(at)) }], seek, reset));
  step(30, 17);
  step(10, 17, true);
  deliver(datedPanel(panel.id, 10, 'current', 17));
  head = 30;
  frame();
  assert.equal(entry.hour, 10);
  assert.equal(panel.panel?.data, 'current');
  store.seek('1', 18);
  head = 6;
  step(6, 17, true);
  deliver(datedPanel(panel.id, 5, 'old seek', 17));
  assert.equal(entry.answer, undefined);
  assert.equal(panel.panel, undefined);
  step(6, 18, true);
  deliver(datedPanel(panel.id, 6, 'new seek', 18));
  assert.equal(entry.hour, 6);
  assert.equal((panel as FollowEntry).panel?.data, 'new seek');
  store.seek('1', 19, 50);
  head = 50;
  assert.equal((panel as FollowEntry).panel, undefined, 'a forward seek beyond confirmed coverage waits for a reading');
  deliver(datedPanel(panel.id, 50, 'replacement', 19));
  assert.equal((panel as FollowEntry).panel?.data, 'replacement');
});

test('a forward scrub keeps the confirmed interval containing the target', () => {
  const { store, add, deliver } = dated(() => 12);
  const entry = add();
  store.bind('1', 9, 1);
  const packet = datedPanel(entry.id, 10, 'still current');
  packet[33] = 3;
  new DataView(packet.buffer).setInt32(40, 22, true);
  deliver(packet);
  store.seek('1', 2, 17);
  assert.deepEqual(store.coverage(entry), { start: 10, through: 22 });
  assert.equal(entry.panel?.data, 'still current');
});

test('an early panel leaves the eligible picture up until its hour arrives', () => {
  let hour = 10;
  const { store, add, deliver, frame } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  deliver(datedPanel(entry.id, 10, 'early'));
  deliver(datedPanel(entry.id, 13, 'later'));
  assert.equal(entry.panel?.data, 'early');
  hour = 13;
  frame();
  assert.equal(entry.panel?.data, 'later');
  store.seek('1', 2);
  deliver(datedPanel(entry.id, 5, 'abandoned generation'));
  assert.equal(entry.panel, undefined);
  deliver(datedPanel(entry.id, 5, 'after seek', 2));
  assert.equal((entry as FollowEntry).panel?.data, 'after seek');
});

test('exact research slot snapshots stay paintable when their live progress stream has passed the read', () => {
  const { store, add, deliver } = dated(() => 100);
  const entry = add();
  store.bind('1', 9, 1);
  const id = 0x80000001;
  const painted: number[] = [];
  for (const hour of [10, 20, 30]) {
    deliver(datedStep(hour + 2, [{ id, hour: hour + 2, answer: one(1n, 90n) }], 1));
    deliver(datedPanel(entry.id, hour, { live: {
      groups: [{ table: 0, columns: [0], keys: ['1'], follow: id, seedHour: hour, values: [hour] }], rows: [], kinds: [], cells: [],
    } }));
    painted.push(store.coverage(entry)!.through);
    assert.equal(store.group(entry, id)?.get(0, 0), hour, 'the exact progress belongs to the painted hour');
  }
  assert.deepEqual(painted, [10, 20, 30]);
});

test('a dated panel and two independently confirmed cell readings meet at their shared coverage', () => {
  let hour = 40;
  const { store, add, deliver, frame } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  const spec = (follow: number, seedHour: number) => ({ table: 0, keys: ['1'], columns: [0], follow, seedHour, values: [12] });
  const groups = [spec(0x80000001, 10), spec(0x80000002, 12)];
  deliver(datedPanel(entry.id, 12, { live: { groups, rows: [], kinds: [], cells: [] } }));
  deliver(datedPanel(entry.id, 30));
  const step = (at: number, id: number, start: number) =>
    deliver(datedStep(at, [{ id, hour: start, answer: one(1n, 12n) }], 1));
  step(22, groups[0]!.follow, 10);
  assert.deepEqual(store.coverage(entry), { start: 12, through: 12 }, 'the second seed confirms only its own hour');
  step(24, groups[1]!.follow, 12);
  assert.deepEqual(store.coverage(entry), { start: 12, through: 24 });
  assert.equal(entry.hour, 12, 'the confirmation leaves the structure start intact');
  assert.equal(entry.through, 30);
  deliver(datedPanel(entry.id, 25, { live: { groups: groups.map(g => ({ ...g, seedHour: 25 })), rows: [], kinds: [], cells: [] } }));
  deliver(datedPanel(entry.id, 40));
  deliver(datedStep(29, [], 1));
  assert.deepEqual(store.coverage(entry), { start: 25, through: 29 }, 'an established stream can confirm a newer exact seed');
  hour = 30;
  step(30, groups[1]!.follow, 34);
  assert.deepEqual(store.coverage(entry), { start: 25, through: 29 }, 'an individual reading ahead of the common lane head waits');
  hour = 34;
  frame();
  assert.deepEqual(store.coverage(entry), { start: 25, through: 29 }, 'the two readings still have no shared hour');
  hour = 35;
  deliver(datedStep(35, [], 1));
  assert.deepEqual(store.coverage(entry), { start: 34, through: 35 });
});

test('a changing value stays in the worker until interaction requests its dated snapshot', () => {
  const decoder = new FollowDecoder(), released: unknown[] = [];
  const followed = new Followed(what => { released.push(what); decoder.release(what); });
  followed.add(3, 'branch');
  followed.bind('branch', 9, 1);
  const group = 0x80000001;
  const live = { groups: [{ table: 0, keys: ['1'], columns: [0], follow: group, seedHour: 10, values: [7] }],
    rows: ['1'], kinds: [{ id: 'progress', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] };
  const land = (packet: Uint8Array, hour: number) => {
    followed.land([decoder.item(packet.slice().buffer)]);
    followed.drain(hour);
    return [...followed.structures(true)];
  };
  assert.equal(land(datedPanel(3, 10, { live }), 10)[0]?.values?.get(liveKey('1', 'progress')), 7);
  const full = datedStep(11, [{ id: group, hour: 10, answer: one(1n, 8n) }], 1);
  assert.deepEqual(land(full, 11), []);
  assert.deepEqual(land(datedPanel(3, 11), 11), []);
  const snapshot = followed.snapshot(3);
  assert.equal(snapshot?.through, 11);
  assert.equal(snapshot?.values?.get(liveKey('1', 'progress')), 8);
  followed.seek('branch', 2);
  assert.equal(followed.snapshot(3)?.panel, null);
  followed.remove(3);
  assert.deepEqual(released, [3, { route: 9 }]);
});

test('a staged product computes its structural result only once', () => {
  const followed = new Followed(() => {});
  followed.add(1, 'a', 'nuclear-arsenal'); followed.bind('a', 1, 1);
  let validations = 0;
  const reply = { missing: null, warnings: [], types: [], get countries() { validations++; return []; } };
  followed.land([{ kind: 'panels', batch: { route: 1, clock: 0, seek: 1, completed: undefined,
    panels: [{ id: 1, hour: 1, stepped: true, panel: { data: reply, refused: null } }] } }], () => {});
  const prepared = validations;
  followed.drain(1);
  [...followed.structures(true)];
  assert.ok(prepared > 0); assert.equal(validations, prepared);
});
