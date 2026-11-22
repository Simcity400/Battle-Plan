import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, cellState, cellValue, parseFollowed, rowHour, rowKeys, rowName, rowReason } from './message.ts';
import type { FollowItem, FollowedAnswer } from './message.ts';
import { datedPanel, fakeTransport, manualFrames, message } from './messages.fixture.ts';
import { BarCell, TextCell, formatAmountFast, paintArmy, paintMeasure, paintStability } from './painters.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { FollowStore } from './store.ts';
import { Followed } from '../../draw/followed.ts';
import type { FollowUpdate } from '../../draw/followed';
import { StructureOutbox } from '../../draw/structureOutbox.ts';
import { LatestReadings, type Show } from './latest.ts';
import { serialize } from 'node:v8';
import { liveKey } from '../../charts/live.ts';
import { DatedStore } from './datedStore.ts';
import { FollowDecoder } from './decode.ts';
import type { FollowEntry, PageReading } from './store.ts';
import type { FollowedPanel } from './message.ts';

const one = (key: bigint, value: bigint) => ({ keys: [key], columns: 1, values: [value] });

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

test('a seek that keeps what covers its hour leaves the choice to the worker, whose coverage is current', () => {
  const transport = fakeTransport();
  const store = new FollowStore(transport, manualFrames().schedule);
  const { entry } = store.acquire({ product: 'series-state', params: { branchId: '1' } });
  const answer = parseFollowed(message(10, [{ id: entry.id, answer: one(1n, 7n) }], false, 9, 1)).answers[0]!.answer;
  transport.reading({ id: entry.id, hour: 10, through: 10, coverage: { start: 10, through: 10 }, answer });
  store.seek('1', 40);
  assert.equal(entry.answer, answer, 'an answer last sent covering hour 10 can still stand at hour 40');
  transport.reading({ id: entry.id, hour: 10, through: 10, answer: null });
  assert.equal(entry.answer, undefined, 'the worker drops it');
  transport.reading({ id: entry.id, hour: 41, through: 41, coverage: { start: 41, through: 41 }, answer });
  store.seek('1');
  assert.equal(entry.answer, undefined, 'a seek that keeps nothing clears at once');

  const { entry: drawn } = store.acquire({ panel: 'nuclear-arsenal', branchId: '1' });
  const landed = () => drawn.landed;
  const step = (hour: number) => transport.reading({ id: drawn.id, hour, through: hour, coverage: { start: hour, through: hour },
    panel: { data: { hour }, refused: null }, values: new Map([['GER:total', hour]]) });
  assert.equal(landed(), undefined, 'nothing has landed on a new follow');
  step(23); step(24);
  transport.reading({ id: drawn.id, hour: 24, through: 24, panel: null, values: null });
  assert.deepEqual([drawn.panel, landed()?.panel.data, landed()?.values?.get('GER:total'), landed()?.hour, landed()?.through],
    [undefined, { hour: 24 }, 24, 24, 24], 'a forward step that empties the reading keeps the last panel to arrive, with its values and hours, whatever rendered in between');
  store.seek('1', 30);
  assert.equal(landed()?.hour, 24, 'a forward seek leaves it to the worker');
  store.seek('1');
  assert.equal(landed(), null, 'a backward seek discards it, and the follow stays one that has landed');
  step(25);
  assert.equal(landed()?.hour, 25);
  store.seek('1', 25, true);
  assert.deepEqual(drawn.panel?.data, { hour: 25 }, 'a paused step to an hour the reading covers keeps it');
  store.seek('1', 26, true);
  assert.deepEqual([drawn.panel, drawn.values, landed()?.hour], [undefined, undefined, 25],
    'a paused step past what the reading covers empties it at once, before the worker says so, and holds the last panel apart');
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
  assert.deepEqual([...bytes.slice(40, 48)], [0x81, 0xa4, 114, 111, 119, 115, 0x91, 7]);
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
  land(completedStep(34));
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

function completedStep(hour: number) {
  const packet = datedPanel(0, hour);
  packet[32] = 4;
  return packet;
}

test('a stepped panel covers each completed step until its next reading arrives', () => {
  let hour = 12;
  const { store, add, deliver, frame } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  const packet = datedPanel(entry.id, 10, 'held');
  packet[33] = 1;
  deliver(packet);
  deliver(completedStep(22));
  deliver(datedPanel(entry.id, 23, 'changed'));
  hour = 22; frame();
  assert.deepEqual(store.coverage(entry), { start: 10, through: 22 });
  assert.equal(entry.panel?.data, 'held');
  hour = 23; frame();
  assert.deepEqual(store.coverage(entry), { start: 23, through: 23 });
  const followed = new Followed(() => {});
  followed.add(1, '1'); followed.bind('1', 9, 1);
  followed.land([{ kind: 'panels', batch: { route: 9, clock: 100, seek: 1, completed: 122,
    panels: [{ id: 1, hour: 110, stepped: true, panel: { data: 'held', refused: null } }] } }]);
  followed.drain(12);
  assert.equal(followed.current(1)?.coverage?.through, 110, 'a completed step stands only once its hour is presented');
  followed.drain(30);
  assert.equal(followed.current(1)?.coverage?.through, 122, 'coverage stops at the completed step on the panel clock');
});

test('a product waiting for its table is covered only through the hour before it waited, whatever later steps complete', () => {
  let hour = 12;
  const { store, add, deliver, frame } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  const packet = datedPanel(entry.id, 10, 'held');
  packet[33] = 1;
  deliver(packet);
  deliver(completedStep(22));
  const waiting = datedPanel(entry.id, 34);
  waiting[32] = 5;
  deliver(waiting);
  deliver(completedStep(34));
  deliver(completedStep(58));
  deliver(datedPanel(entry.id, 82, 'changed'));
  hour = 58; frame();
  assert.deepEqual(store.coverage(entry), { start: 10, through: 33 });
  hour = 82; frame();
  assert.deepEqual(store.coverage(entry), { start: 82, through: 82 });
});

test('a native reset keeps an old picture to the hours it was confirmed through until its replacement shows', () => {
  let hour = 10;
  const { store, add, deliver, frame } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  const packet = datedPanel(entry.id, 10, 'held');
  packet[33] = 1;
  deliver(packet);
  const waiting = (at: number) => { const wait = datedPanel(entry.id, at); wait[32] = 5; deliver(wait); };
  waiting(34);
  deliver(completedStep(34));
  hour = 58; frame();
  assert.deepEqual(store.coverage(entry), { start: 10, through: 33 });
  deliver(datedStep(58, [], 1, true));
  waiting(58);
  deliver(completedStep(58));
  const group = { table: 0, keys: ['1'], columns: [0], follow: 0x80000001, seedHour: 99, values: [12] };
  const retry = datedPanel(entry.id, 58, { live: { groups: [group], rows: [], kinds: [], cells: [] } });
  retry[33] = 1;
  deliver(retry);
  deliver(completedStep(58));
  hour = 58; frame();
  const covered = store.coverage(entry);
  assert.ok(covered === null || covered.through <= 33, `hours the reset left unread stay uncovered while the replacement waits for its cells: ${JSON.stringify(covered)}`);
});

test('a lane reading answered again after a native reset advances with its lane', () => {
  let hour = 10;
  const { store, add, deliver } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  deliver(datedStep(10, [{ id: entry.id, answer: one(1n, 7n) }], 1));
  hour = 20;
  deliver(datedStep(20, [{ id: entry.id, answer: one(1n, 7n) }], 1, true));
  hour = 34;
  deliver(datedStep(34, [], 1));
  assert.deepEqual(store.coverage(entry), { start: 20, through: 34 });
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

test('a scrub keeps the confirmed interval containing the target', () => {
  const { store, add, deliver, frame } = dated(() => 22);
  const entry = add();
  store.bind('1', 9, 1);
  const packet = datedPanel(entry.id, 10, 'still current');
  packet[33] = 1;
  deliver(packet);
  deliver(completedStep(22));
  frame();
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

test('a pause keeps a stepped panel whose followed cells cover the paused hour, at the paused hour', () => {
  let hour = 10;
  const { store, add, deliver, frame } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  const group = { table: 0, keys: ['1'], columns: [0], follow: 0x80000001, seedHour: 10, values: [5] };
  const panel = datedPanel(entry.id, 10, { live: { groups: [group], rows: [], kinds: [], cells: [] } });
  panel[33] = 1;
  deliver(panel);
  deliver(datedStep(12, [{ id: group.follow, hour: 10, answer: one(1n, 5n) }], 1));
  for (let at = 13; at <= 40; at++) {
    deliver(datedStep(at, at === 15 ? [{ id: group.follow, answer: one(1n, 7n) }] : [], 1));
    deliver(completedStep(at));
  }
  hour = 20; frame();
  const playing = store.coverage(entry)!;
  assert.ok(playing.start <= 20 && 20 <= playing.through, `playing covers the clock: ${JSON.stringify(playing)}`);
  store.seek('1', 1, 20);
  frame();
  const paused = store.coverage(entry);
  assert.ok(paused && paused.start <= 20 && 20 <= paused.through, `the pause keeps the hour it stops at: ${JSON.stringify(paused)}`);
  const cells = store.group(entry, group.follow);
  assert.equal(cells?.get(0, 0), 7, 'the values at the paused hour, never the seed');
  assert.ok(cells?.landedHour != null && cells.landedHour <= 20, `landed from an answer at or before the pause: ${cells?.landedHour}`);
  deliver(datedStep(20, [{ id: group.follow, answer: one(1n, 7n) }], 1, true));
  frame();
  const reset = store.coverage(entry);
  assert.ok(reset && reset.start <= 20 && 20 <= reset.through, `native's reseek at the paused hour keeps it: ${JSON.stringify(reset)}`);
  const again = datedPanel(entry.id, 20, { live: { groups: [{ ...group, seedHour: 20, values: [7] }], rows: [], kinds: [], cells: [] } });
  again[33] = 1;
  deliver(again);
  frame();
  const resent = store.coverage(entry);
  assert.ok(resent && resent.start <= 20 && 20 <= resent.through, `the panel native sends again at the paused hour keeps it: ${JSON.stringify(resent)}`);
});

test('a pause just ahead of the drawn hour keeps what already arrived for it, and drops only what lies beyond', () => {
  let hour = 10;
  const { store, add, deliver, frame } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  const group = { table: 0, keys: ['1'], columns: [0], follow: 0x80000001, seedHour: 10, values: [5] };
  const panel = datedPanel(entry.id, 10, { live: { groups: [group], rows: [], kinds: [], cells: [] } });
  panel[33] = 1;
  deliver(panel);
  deliver(datedStep(12, [{ id: group.follow, hour: 10, answer: one(1n, 5n) }], 1));
  for (let at = 13; at <= 40; at++) {
    deliver(datedStep(at, at === 19 ? [{ id: group.follow, answer: one(1n, 7n) }] : at === 30 ? [{ id: group.follow, answer: one(1n, 9n) }] : [], 1));
    deliver(completedStep(at));
  }
  hour = 18; frame();
  store.seek('1', 1, 20);
  const paused = store.coverage(entry);
  assert.ok(paused && paused.start <= 20 && 20 <= paused.through, `the pause keeps the hour it stops at: ${JSON.stringify(paused)}`);
  assert.equal(store.group(entry, group.follow)?.get(0, 0), 7, 'the value that arrived for hour 19, never one from beyond the pause');
});

test('a pause on a track offset from the axis applies what arrived up to its hour on the track', () => {
  const { store, add, deliver } = dated(() => 0);
  const entry = add();
  store.bind('1', 9, 1);
  const clocked = (bytes: Uint8Array, at: number) => { new DataView(bytes.buffer, bytes.byteOffset).setInt32(at, 100, true); return bytes; };
  const group = { table: 0, keys: ['1'], columns: [0], follow: 0x80000001, seedHour: 110, values: [5] };
  const panel = clocked(datedPanel(entry.id, 110, { live: { groups: [group], rows: [], kinds: [], cells: [] } }), 12);
  panel[33] = 1;
  deliver(panel);
  deliver(clocked(datedStep(112, [{ id: group.follow, hour: 110, answer: one(1n, 5n) }], 1), 20));
  for (let at = 113; at <= 140; at++) {
    deliver(clocked(datedStep(at, at === 119 ? [{ id: group.follow, answer: one(1n, 7n) }] : at === 130 ? [{ id: group.follow, answer: one(1n, 9n) }] : [], 1), 20));
    deliver(clocked(completedStep(at), 12));
  }
  store.seek('1', 1, 120);
  const paused = store.coverage(entry);
  assert.ok(paused && paused.start <= 120 && 120 <= paused.through, `the pause keeps the track hour it stops at, nothing drawn before it: ${JSON.stringify(paused)}`);
  assert.equal(store.group(entry, group.follow)?.get(0, 0), 7, 'the value that arrived for track hour 119, never the one for 130');
});

test('a pause keeps a stepped panel when native sends the panel again before its cells', () => {
  let hour = 10;
  const { store, add, deliver, frame } = dated(() => hour);
  const entry = add();
  store.bind('1', 9, 1);
  const group = { table: 0, keys: ['1'], columns: [0], follow: 0x80000001, seedHour: 10, values: [5] };
  const panel = datedPanel(entry.id, 10, { live: { groups: [group], rows: [], kinds: [], cells: [] } });
  panel[33] = 1;
  deliver(panel);
  deliver(datedStep(12, [{ id: group.follow, hour: 10, answer: one(1n, 5n) }], 1));
  for (let at = 13; at <= 40; at++) {
    deliver(datedStep(at, at === 15 ? [{ id: group.follow, answer: one(1n, 7n) }] : [], 1));
    deliver(completedStep(at));
  }
  hour = 20; frame();
  const playing = store.coverage(entry)!;
  assert.ok(playing.start <= 20 && 20 <= playing.through, `playing covers the clock: ${JSON.stringify(playing)}`);
  store.seek('1', 1, 20);
  frame();
  const paused = store.coverage(entry);
  assert.ok(paused && paused.start <= 20 && 20 <= paused.through, `the pause keeps the hour it stops at: ${JSON.stringify(paused)}`);
  const cells = store.group(entry, group.follow);
  assert.equal(cells?.get(0, 0), 7, 'the values at the paused hour, never the seed');
  assert.ok(cells?.landedHour != null && cells.landedHour <= 20, `landed from an answer at or before the pause: ${cells?.landedHour}`);
  const again = datedPanel(entry.id, 20, { live: { groups: [{ ...group, seedHour: 20, values: [7] }], rows: [], kinds: [], cells: [] } });
  again[33] = 1;
  deliver(again);
  frame();
  const early = store.coverage(entry);
  assert.ok(early && early.start <= 20 && 20 <= early.through, `the panel sent again first keeps it: ${JSON.stringify(early)}`);
  deliver(datedStep(20, [{ id: group.follow, answer: one(1n, 7n) }], 1, true));
  frame();
  const resent = store.coverage(entry);
  assert.ok(resent && resent.start <= 20 && 20 <= resent.through, `the panel native sends again at the paused hour keeps it: ${JSON.stringify(resent)}`);
});

test('a stepped panel whose cells are followed stands through each completed step that sends nothing of it', () => {
  const decoder = new FollowDecoder();
  const followed = new Followed(what => decoder.release(what));
  followed.add(3, 'branch');
  followed.bind('branch', 9, 1);
  const group = 0x80000001;
  const live = { groups: [{ table: 0, keys: ['1'], columns: [0], follow: group, seedHour: 10, values: [7] }],
    rows: ['1'], kinds: [{ id: 'progress', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] };
  const step = (completed: number, panels: { id: number; hour: number; stepped: boolean; panel: { data: unknown; refused: null } }[]) =>
    followed.land([{ kind: 'panels', batch: { route: 9, clock: 0, seek: 1, completed, panels } }]);
  step(10, [{ id: 3, hour: 10, stepped: true, panel: { data: { live }, refused: null } }]);
  followed.drain(10);
  assert.equal(followed.current(3)?.coverage?.through, 10);
  followed.land([decoder.item(datedStep(11, [{ id: group, hour: 11, answer: one(1n, 8n) }], 1).slice().buffer)]);
  step(11, []);
  followed.drain(11);
  assert.equal(followed.current(3)?.coverage?.through, 11, 'a completed step sends each due product that changed');
  assert.equal(followed.current(3)?.values?.get(liveKey('1', 'progress')), 8);
});

test('a shown panel stands only until the hour a new panel waiting on its first cell answers starts', () => {
  const decoder = new FollowDecoder();
  const followed = new Followed(what => decoder.release(what));
  followed.add(3, 'branch');
  followed.bind('branch', 9, 1);
  const first = 0x80000001, second = 0x80000002;
  const live = (follow: number, seed: boolean) => ({ groups: [{ table: 0, keys: ['1'], columns: [0], follow, ...seed ? { seedHour: 10, values: [7] } : {} }],
    rows: ['1'], kinds: [{ id: 'progress', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] });
  const step = (completed: number, panels: { id: number; hour: number; stepped: boolean; panel: { data: unknown; refused: null } }[]) =>
    followed.land([{ kind: 'panels', batch: { route: 9, clock: 0, seek: 1, completed, panels } }]);
  step(10, [{ id: 3, hour: 10, stepped: true, panel: { data: { live: live(first, true) }, refused: null } }]);
  followed.drain(10);
  followed.land([decoder.item(datedStep(11, [{ id: first, hour: 11, answer: one(1n, 8n) }], 1).slice().buffer)]);
  step(11, []);
  followed.drain(11);
  step(12, [{ id: 3, hour: 12, stepped: true, panel: { data: { live: live(second, false) }, refused: null } }]);
  followed.land([decoder.item(datedStep(14, [{ id: first, hour: 14, answer: one(1n, 9n) }], 1).slice().buffer)]);
  step(14, []);
  followed.drain(14);
  assert.equal(followed.current(3)?.coverage?.through, 11, 'the old panel is not the page after the new one starts');
  const third = 0x80000003;
  step(20, [{ id: 3, hour: 20, stepped: true, panel: { data: { live: live(third, false) }, refused: null } }]);
  followed.land([decoder.item(datedStep(20, [{ id: first, hour: 20, answer: one(1n, 10n) }], 1).slice().buffer)]);
  followed.drain(20);
  assert.equal(followed.current(3)?.coverage?.through, 11, 'a later waiting panel never moves the end of the shown one');
});

type Answers = Parameters<typeof message>[1];
interface Seen { panel?: FollowedPanel | null; answer?: FollowedAnswer | null; values?: Map<string, number | string> | null; hour: number; coverage?: { start: number; through: number } }

const describe = (answer: FollowedAnswer) => answer.kind === 'notRecorded' ? answer.reason
  : `${answer.kind}:${rowKeys(answer)}:${Array.from({ length: answer.rows * answer.columns }, (_, at) => String(cellValue(answer, at))).join(',')}`;
const contentOf = (r: Seen | undefined) => !r || !r.panel && !r.answer ? 'empty'
  : JSON.stringify([r.panel ? [r.panel.refused, r.panel.data] : null, r.answer ? describe(r.answer) : null, r.values ? [...r.values] : null, r.hour, r.coverage?.start ?? null]);
const covering = (r: Seen | undefined, hour: number) => !r || !r.panel && !r.answer || !!r.coverage && r.coverage.start <= hour && hour <= r.coverage.through;
const CELLS = 0x80000001;
const cellPanel = (seedHour: number, value: number) => ({ live: { groups: [{ table: 0, keys: ['1'], columns: [0], follow: CELLS, seedHour, values: [value] }],
  rows: ['1'], kinds: [{ id: 'progress', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] } });

function pipeline(ids = [3], horizon = 0) {
  const decoders = [new FollowDecoder(), new FollowDecoder()];
  const worker = new Followed(what => decoders[0]!.release(what)), reference = new Followed(what => decoders[1]!.release(what));
  for (const followed of [worker, reference]) { for (const id of ids) followed.add(id, 'branch'); followed.bind('branch', 9, 1); }
  let seek = 1, epoch = 0, told = true, holding = false, playing = true, hour = 0, head = 0, recorded = -1, epochFrom = 0, failed = false;
  const history = new Map<number, Map<number, { content: string; covers: boolean; through?: number }>>(ids.map(id => [id, new Map()]));
  const known = new Map<number, Map<string, { start: number; through: number }[]>>(ids.map(id => [id, new Map()]));
  const learn = (id: number, content: string, coverage: { start: number; through: number } | undefined) => {
    if (content === 'empty' || !coverage) return;
    const spans = known.get(id)!;
    let held = spans.get(content);
    if (!held) spans.set(content, held = []);
    if (!held.some(span => span.start === coverage.start && span.through === coverage.through)) held.push({ ...coverage });
  };
  const order = [...ids];
  let receive: (reading: PageReading, quiet: boolean) => void = () => {}, fail: (reason: string) => void = () => {};
  const store = new FollowStore({ follow: () => {}, unfollow: () => {}, listen: (r, f) => { receive = r; fail = f; return () => {}; } },
    manualFrames().schedule, () => order.shift()!);
  const entries = new Map(ids.map(id => [id, store.acquire({ q: id, branchId: 'branch' }).entry]));
  const page = new LatestReadings(id => ids.includes(id) && told ? epoch : undefined);
  const shown: string[] = [], posted: FollowUpdate[][] = [], channel: FollowUpdate[][] = [];
  const label = (r: Seen) => `${r.hour}:${r.panel?.refused ?? (r.panel ? 'panel' : r.answer?.kind === 'notRecorded' ? r.answer.reason : r.answer?.kind ?? 'pending')}`;
  const show: Show = (r, quiet) => { receive(r, quiet); if (!quiet) shown.push(ids.length > 1 ? `${r.id}@${label(r)}` : label(r)); };
  const outbox = new StructureOutbox(() => {
    const updates = [...worker.updates(head + (playing ? horizon : 0))];
    if (updates.length) { channel.push(updates); posted.push(updates); }
    return updates.length > 0;
  });
  const deliver = () => {
    const updates = channel.shift();
    if (!updates) return false;
    for (const update of updates) page.land(update, hour, !playing, show);
    outbox.taken();
    return true;
  };
  const settle = () => { if (!holding) while (deliver()); };
  const note = (at: number) => {
    reference.drain(at);
    for (const id of ids) {
      const r = reference.current(id), content = contentOf(r);
      history.get(id)!.set(at, { content, covers: covering(r, at), through: r?.coverage?.through });
      learn(id, content, r?.coverage);
      for (const lead of reference.ahead(id)) {
        const coverage = { start: lead.hour, through: lead.hour };
        learn(id, contentOf({ panel: lead.panel, hour: lead.hour, coverage }), coverage);
      }
    }
  };
  const record = (to: number) => {
    for (let at = recorded + 1; at <= to; at++) note(at);
    recorded = Math.max(recorded, to);
  };
  const tick = (at: number) => { head = at; record(at); worker.drain(at); outbox.flush(); settle(); };
  const arrive = (items: FollowItem[]) => { worker.land(items); reference.land(items); note(recorded); tick(head); };
  const moved = (sought: boolean, retainThrough: number | undefined) => {
    if (sought) seek++;
    epoch++; told = true;
    page.forget(ids); store.seek('branch', retainThrough, true);
    worker.seek('branch', seek, epoch, retainThrough); reference.seek('branch', seek, epoch, retainThrough);
    if (!sought) note(recorded);
  };
  const check = (where: string) => {
    for (const [id, entry] of entries) {
      if (failed) {
        if (entry.status !== 'unavailable' || entry.panel || entry.answer) assert.fail(`${where}: subscription ${id} shows data after the stream failed`);
        continue;
      }
      const content = contentOf(entry);
      if (content === 'empty') continue;
      if (entry.through > hour || (entry.coverage?.start ?? -Infinity) > hour)
        assert.fail(`${where}: subscription ${id} shows ${label(entry)} through ${entry.through}, ahead of the page at ${hour}`);
      if (!playing && !covering(entry, hour)) assert.fail(`${where}: paused at ${hour}, subscription ${id} shows ${label(entry)}, which does not cover it, as current`);
      const spans = known.get(id)!.get(content) ?? [];
      if (entry.through >= epochFrom && !spans.some(span => span.start <= entry.through && entry.through <= span.through))
        assert.fail(`${where}: subscription ${id} dates ${content} at ${entry.through}, which no direct read ever held at that hour`);
    }
  };
  const exact = (where: string) => {
    if (failed) { check(where); return; }
    for (const [id, entry] of entries) {
      const truth = history.get(id)!.get(hour)!;
      const expected = !playing && !truth.covers ? 'empty' : truth.content;
      if (contentOf(entry) !== expected)
        assert.fail(`${where}: at ${hour} subscription ${id} shows ${contentOf(entry)}, a direct read gives ${expected}${expected === 'empty' ? ' (held, uncovered)' : ''}`);
      const through = truth.through === undefined ? undefined : Math.min(truth.through, hour);
      if (expected !== 'empty' && through !== undefined && entry.through !== through)
        assert.fail(`${where}: at ${hour} subscription ${id} is dated through ${entry.through}, a direct read through ${through}`);
    }
    check(where);
  };
  record(0);
  const panels = (id: number, at: number, data: unknown, refused: string | null, stepped: boolean) => arrive([{ kind: 'panels', batch: { route: 9, clock: 0, seek,
    completed: undefined, panels: [{ id, hour: at, stepped, panel: { data, refused } }] } }]);
  const lane = (bytes: Uint8Array) => {
    const items = decoders.map(decoder => decoder.item(bytes.slice().buffer));
    worker.land([items[0]!]); reference.land([items[1]!]); note(recorded); tick(head);
  };
  return {
    worker, entries, page, shown, posted, channel, label,
    get head() { return head; }, get hour() { return hour; }, get playing() { return playing; },
    panel: (at: number, refused: string | null, stepped = true, id = ids[0]!) => panels(id, at, { rows: [at] }, refused, stepped),
    data: (at: number, data: unknown, id = ids[0]!, stepped = true) => panels(id, at, data, null, stepped),
    step: (at: number, answers: Answers) => lane(datedStep(at, answers, seek)),
    completed: (at: number) => arrive([{ kind: 'panels', batch: { route: 9, clock: 0, seek, completed: at, panels: [] } }]),
    reset: (at: number, answers: Answers = []) => lane(datedStep(at, answers, seek, true)),
    fail: () => {
      const item: FollowItem = { kind: 'failure', reason: 'stream closed' };
      worker.land([item]); reference.land([item]); page.forget(ids); fail('stream closed'); failed = true; tick(head);
    },
    frame: (at: number) => tick(at),
    view: (at: number) => { hour = at; record(at); page.advance(at, !playing, show); },
    pause: () => { playing = false; moved(false, hour); page.refresh(hour, true, show); tick(head); },
    resume: () => { playing = true; page.refresh(hour, false, show); },
    seekTo: (to: number, forward: boolean) => {
      playing = false; told = false; hour = to;
      const before = shown.length;
      page.advance(to, true, show);
      if (shown.length !== before) assert.fail(`a frame listener told of the seek to ${to} before its anchor showed ${shown.slice(before)}`);
      moved(true, forward ? to : undefined);
      hour = to; epochFrom = to; recorded = to - 1;
      for (const spans of known.values()) spans.clear();
      record(to);
      page.refresh(hour, true, show); tick(to); page.advance(to, true, show);
    },
    unanchored: () => { told = false; },
    deliver, hold: (held: boolean) => { holding = held; if (!held) settle(); },
    check, exact,
    sync: (where: string) => { holding = false; tick(head); hour = head; record(hour); page.advance(hour, !playing, show); settle(); exact(where); },
  };
}

test('the page shows, for its own hour, the latest reading that is due, each sent once and replaced by its due hour', () => {
  let seek = 5;
  const page = new LatestReadings(() => seek), shown: string[] = [];
  const show: Show = (r, quiet) => { if (!quiet) shown.push(`${r.id}@${r.hour}:${r.through}`); };
  const at = (due: number) => ({ due, hour: due, coverage: { start: due, through: due }, panel: { refused: String(due) }, answer: null, values: null });
  page.land({ id: 1, epoch: 5, keep: 10, put: [at(10), at(12), at(20)] }, 4, false, show);
  assert.deepEqual(shown, [], 'nothing is due at hour 4, so the page shows nothing from hour 10');
  page.advance(11, false, show);
  page.advance(15, false, show);
  assert.deepEqual(shown, ['1@10:10', '1@12:12']);
  page.land({ id: 1, epoch: 5, keep: 12, cover: [[12, 18]], drop: [20], put: [at(24)] }, 15, false, show);
  page.advance(22, false, show);
  assert.deepEqual(shown, ['1@10:10', '1@12:12'], 'a coverage patch dates the shown reading quietly; a dropped lead never shows');
  page.advance(24, false, show);
  assert.deepEqual(shown.at(-1), '1@24:24');
  seek = 6;
  page.land({ id: 1, epoch: 5, keep: 30, put: [at(30)] }, 30, false, show);
  page.advance(40, false, show);
  assert.equal(shown.at(-1), '1@24:24', 'nothing of an earlier seek is shown, on landing or on a frame');
});

test('a reading the worker drained ahead of the page waits for its hour, and the page never dates anything past its own hour', () => {
  const p = pipeline([6]);
  p.step(0, [{ id: 6, answer: one(1n, 1n) }]);
  p.sync('start');
  for (let at = 1; at <= 24; at++) p.step(at, at === 24 ? [{ id: 6, answer: one(1n, 24n) }] : []);
  p.frame(24);
  p.view(4); p.check('page at 4, worker through 24');
  assert.equal(p.entries.get(6)!.through <= 4 && describe(p.entries.get(6)!.answer!), 'rows:1:1');
  p.view(24); p.exact('page at 24');
});

test('a changing value with an unchanged structure reaches the page at every step', () => {
  const p = pipeline([6]);
  for (let at = 0; at <= 96; at += 24) {
    p.step(at, [{ id: 6, answer: one(1n, BigInt(7 + at)) }]);
    p.frame(at); p.view(at); p.exact(`hour ${at}`);
  }
  assert.equal(describe(p.entries.get(6)!.answer!), 'rows:1:103');
});

test('a panel whose followed cells change reaches the page with each new value', () => {
  const p = pipeline([7]);
  p.data(0, cellPanel(0, 5), 7, false);
  for (let at = 1; at <= 30; at += 3) { p.step(at, [{ id: CELLS, answer: one(1n, BigInt(at)) }]); p.completed(at); p.sync(`hour ${at}`); }
});

test('resuming after a paused reading short of its hour never puts an older roster over a newer one', () => {
  const p = pipeline([3], 100);
  p.panel(23, 'A');
  p.frame(23); p.view(23); p.frame(24); p.view(24); p.pause();
  p.exact('paused at 24 with only the hour-23 roster');
  p.panel(90, 'C');
  p.panel(24, 'B');
  p.resume();
  p.frame(25); p.view(25); p.exact('playing at 25');
  for (const at of [60, 89, 90, 91]) { p.frame(at); p.view(at); p.exact(`playing at ${at}`); }
  assert.equal(p.entries.get(3)!.panel?.refused, 'C');
});

test('a paused reading short of the paused hour is held until its coverage reaches it, then shown once', () => {
  const p = pipeline([3, 6]);
  p.panel(23, 'A', true, 3);
  p.step(23, [{ id: 6, answer: one(1n, 7n) }]);
  p.frame(23); p.view(23); p.frame(24); p.view(24); p.pause();
  p.exact('paused at 24');
  assert.deepEqual([p.entries.get(3)!.panel, p.entries.get(3)!.landed?.panel.refused, p.entries.get(6)!.answer], [undefined, 'A', undefined],
    'the panel is held apart, to be dimmed, and the answer is missing');
  p.panel(23, 'A', true, 3);
  p.step(24, [{ id: 6, hour: 23, answer: one(1n, 7n) }]);
  p.exact('native sends the paused hour again');
  const before = p.shown.length;
  p.completed(24);
  p.exact('coverage reaches 24');
  assert.deepEqual(p.shown.slice(before), ['3@23:A']);
  p.completed(24); p.step(24, []);
  assert.equal(p.shown.length, before + 1, 'only once');
});

test('a paused correction that never reached the page still arrives after a resume', () => {
  const p = pipeline([3, 6]);
  p.step(23, [{ id: 6, answer: one(1n, 7n) }]);
  p.frame(23); p.view(23); p.frame(24); p.view(24); p.pause();
  p.hold(true);
  p.panel(24, 'X', false, 3);
  p.step(24, [{ id: 6, answer: one(1n, 9n) }]);
  p.resume();
  p.hold(false);
  p.sync('after the resume');
  assert.equal(describe(p.entries.get(6)!.answer!), 'rows:1:9');
});

test('a forward seek publishes nothing queued from the previous seek, even to a frame that runs before the seek is told', () => {
  const p = pipeline([3], 100);
  p.panel(10, 'old');
  p.panel(30, 'queued');
  p.frame(10); p.view(10);
  p.unanchored();
  p.view(30);
  assert.notEqual(p.entries.get(3)!.panel?.refused, 'queued', `a frame between the seek and its anchor shows nothing of the old seek: ${p.shown}`);
});

test('a paused forward seek dates the new reading by its own coverage', () => {
  const p = pipeline([3]);
  p.panel(20, 'O', false);
  p.frame(23); p.view(23); p.pause();
  p.panel(24, 'P', false);
  p.seekTo(24, true);
  p.exact('after the seek to 24');
  const entry = p.entries.get(3)!;
  assert.deepEqual([entry.panel?.refused, entry.through, entry.coverage], ['P', 24, { start: 24, through: 24 }]);
});

test('a stream failure drops what arrived before the subscription was bound', () => {
  const followed = new Followed(() => {});
  followed.add(3, 'branch');
  followed.land([{ kind: 'panels', batch: { route: 9, clock: 0, seek: 1, completed: undefined,
    panels: [{ id: 3, hour: 10, stepped: true, panel: { data: { rows: [10] }, refused: 'before' } }] } }]);
  followed.land([{ kind: 'failure', reason: 'stream closed' }]);
  followed.bind('branch', 9, 1);
  followed.drain(10);
  assert.equal(followed.current(3)?.panel, null);
});

test('a seek or a stream failure forgets what the page holds, so the same reading after it goes again', () => {
  const p = pipeline([3, 4]);
  p.panel(10, 'A', false, 3);
  p.frame(10); p.view(10);
  p.hold(true);
  p.panel(10, 'X', false, 4);
  p.seekTo(10, true);
  p.panel(10, 'A', false, 3);
  p.hold(false);
  p.sync('after the seek');
  assert.equal(p.entries.get(3)!.panel?.refused, 'A');
});

test('a native reset replaces what it supersedes, on every path', () => {
  const p = pipeline([3, 6]);
  p.panel(10, 'old', true, 3); p.sync('10');
  p.panel(20, 'abandoned', true, 3);
  p.reset(18, [{ id: 6, hour: 25, answer: one(1n, 7n) }]);
  p.panel(18, 'replacement', true, 3);
  for (const at of [18, 20, 25, 26]) { p.frame(at); p.sync(`hour ${at}`); }
  const q = pipeline([3]);
  q.panel(10, 'A'); q.sync('A');
  q.panel(20, 'B'); q.frame(20); q.view(20);
  q.reset(20); q.panel(20, 'A');
  q.sync('back to A');
  assert.equal(q.entries.get(3)!.panel?.refused, 'A');
  const r = pipeline([6]);
  r.step(10, [{ id: 6, answer: one(1n, 7n) }]); r.sync('10');
  r.step(20, [{ id: 6, answer: { notRecorded: 'Twenty' } }]);
  r.reset(25, [{ id: 6, hour: 18, answer: { notRecorded: 'Corrected from 18' } }]);
  r.frame(20); r.view(20); r.check('20');
  r.frame(25); r.sync('25');
  assert.deepEqual(r.entries.get(6)!.answer, { kind: 'notRecorded', reason: 'Corrected from 18' });
});

test('a reset while paused sends its correction, which covers the paused hour', () => {
  const p = pipeline([6]);
  p.step(20, [{ id: 6, answer: one(1n, 7n) }]);
  p.frame(20); p.view(20); p.pause();
  p.reset(20, [{ id: 6, hour: 18, answer: { notRecorded: 'Corrected from 18' } }]);
  p.exact('corrected while paused');
  assert.equal(p.shown.at(-1), '18:Corrected from 18');
});

test('a steady frame at 1,440 h/s sends each new reading once, and 114 subscriptions of 128-country panels stay small', () => {
  const ids = Array.from({ length: 114 }, (_, i) => i + 1), speed = 1440, fps = 60, step = speed / fps;
  const decoder = new FollowDecoder(), followed = new Followed(what => decoder.release(what));
  for (const id of ids) followed.add(id, 'b');
  followed.bind('b', 9, 1);
  const arsenal = (day: number) => ({ missing: null, warnings: [], types: [1, 2, 3].map(ordinal => ({ ordinal, token: `nuke_${ordinal}`, name: `Nuke ${ordinal}` })),
    countries: Array.from({ length: 128 }, (_, i) => ({ tag: `C${i}`, countryId: i, total: i * 3 + day, counts: [1, 2, 3].map(ordinal => ({ ordinal, value: i + ordinal + day, ready: i })) })) });
  const landDays = (from: number, to: number) => {
    for (let day = from; day <= to; day++) for (const id of ids) {
      const packet = datedPanel(id, day * 24, arsenal(day));
      packet[33] = 1;
      followed.land([decoder.item(packet.slice().buffer)]);
    }
  };
  landDays(0, 30);
  followed.drain(0);
  const first = [...followed.updates(0 + step * 2)];
  const sizes: number[] = [], readings: number[] = [];
  let head = 0, day = 30;
  const held = new Map(ids.map(id => [id, new Set(first.find(u => u.id === id)?.put?.map(r => r.due))]));
  for (let frame = 0; frame < 120; frame++) {
    head += step;
    while (day * 24 < head + 720) landDays(++day, day);
    followed.drain(head);
    const updates = [...followed.updates(head + step * 2)];
    for (const update of updates) for (const reading of update.put ?? []) {
      const seen = held.get(update.id)!;
      if (seen.has(reading.due)) assert.fail(`frame ${frame}: subscription ${update.id} was sent its reading for ${reading.due} again`);
      seen.add(reading.due);
    }
    sizes.push(serialize({ type: 'structure', updates }).byteLength);
    readings.push(updates.reduce((n, u) => n + (u.put?.length ?? 0), 0));
  }
  sizes.sort((a, b) => a - b); readings.sort((a, b) => a - b);
  const p95 = sizes[Math.floor(sizes.length * 0.95)]!;
  assert.ok(serialize({ type: 'structure', updates: first }).byteLength < 114 * 3 * 16 * 1024, 'the first message holds at most the readings due within two frames');
  assert.ok(readings.at(-1)! <= 114 * 2, `at most two new readings per subscription a frame: ${readings.at(-1)}`);
  assert.ok(p95 < 2 * 1024 * 1024, `a frame's message stays under 2 MiB: ${(p95 / 1024).toFixed(0)} KiB`);
});

test('random interleavings agree every frame with a direct read at the page hour, and are exact whenever the pipeline is caught up', () => {
  let seed = 0x2545f491;
  const random = (n: number) => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return (seed >>> 0) % n; };
  for (let run = 0; run < 400; run++) {
    const ids = [3, 4, 5, 6, 7], p = pipeline(ids, random(3) * 8);
    let value = 0;
    const answer = () => random(5) ? one(1n, BigInt(++value)) : { notRecorded: `none ${value}` };
    const confirm = () => { p.completed(p.head); p.step(p.head, random(3) ? [] : [{ id: CELLS, answer: one(1n, BigInt(++value)) }]); };
    const resend = () => {
      p.panel(Math.max(0, p.head - random(3)), 'ABC'[random(3)]!, true, 3 + random(2));
      p.step(p.head, [{ id: 6, hour: Math.max(0, p.head - random(3)), answer: answer() }]);
      if (random(2)) confirm();
    };
    for (let op = 0; op < 80; op++) {
      const kind = random(24), where = `run ${run} op ${op} (${kind})`;
      if (kind < 3) p.panel(p.head + random(10), 'ABC'[random(3)]!, true, 3 + random(2));
      else if (kind === 3) p.panel(p.head + random(6), 'XY'[random(2)]!, false, 5);
      else if (kind < 6) p.step(p.head + random(4), [{ id: 6, answer: answer() }, ...random(2) ? [{ id: CELLS, answer: one(1n, BigInt(++value)) }] : []]);
      else if (kind === 6) p.completed(p.head + random(4));
      else if (kind === 7) p.data(p.head + random(4), cellPanel(p.head, ++value), 7, random(2) === 0);
      else if (kind < 11) { if (p.playing) p.frame(p.head + random(5)); if (random(2)) confirm(); }
      else if (kind < 13) p.view(p.playing ? Math.min(p.head, p.hour + random(6)) : p.hour);
      else if (kind === 13) p.hold(random(2) === 0);
      else if (kind < 17) p.sync(where);
      else if (kind === 17) p.reset(p.head + random(3), random(2) ? [{ id: 6, hour: Math.max(0, p.head - random(4)), answer: answer() }] : []);
      else if (kind === 18) {
        const forward = random(2) === 0;
        p.seekTo(forward ? p.head + 1 + random(3) : Math.max(0, p.hour - 1 - random(10)), forward);
        if (random(3)) resend();
      } else if (kind < 21) {
        if (p.playing) { p.frame(p.head); p.view(p.head); p.pause(); if (random(2)) resend(); } else p.resume();
      } else if (kind === 21 && random(10) === 0) p.fail();
      else if (p.playing) p.frame(p.head + random(3));
      p.check(where);
    }
    p.sync(`run ${run} end`);
  }
});
