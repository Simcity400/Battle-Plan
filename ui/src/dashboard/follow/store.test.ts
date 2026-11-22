import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, cellState, cellValue, parseFollowed, rowHour, rowKeys, rowName, rowReason } from './message.ts';
import type { FollowedAnswer } from './message.ts';
import { datedPanel, fakeTransport, manualFrames, message } from './messages.fixture.ts';
import { BarCell, TextCell, formatAmountFast, paintArmy, paintMeasure, paintStability } from './painters.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { FollowStore } from './store.ts';
import { Followed } from '../../draw/followed.ts';
import { liveKey } from '../../charts/live.ts';
import { DatedStore } from './datedStore.ts';
import { FollowDecoder } from './decode.ts';
import type { FollowEntry } from './store.ts';
import { panels, pipeline, VALUE, type Content } from './pipeline.fixture.ts';

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

test('the page store shows what the readings place: current, held apart with its values, dated quietly, emptied', () => {
  const transport = fakeTransport();
  const store = new FollowStore(transport, manualFrames().schedule);
  const { entry } = store.acquire({ panel: 'nuclear-arsenal', branchId: '1' });
  let structure = 0, live = 0;
  store.watch(entry, () => structure++);
  store.watchLive(entry, () => live++);
  const reading = (hour: number, through = hour) => ({ id: entry.id, hour, through, coverage: { start: hour, through }, structure: `S${hour}`,
    panel: { data: { hour }, refused: null }, values: new Map([['GER:total', hour]]) });
  const landed = () => entry.landed;
  assert.equal(landed(), undefined, 'nothing has landed on a new follow');
  transport.reading(reading(23));
  assert.deepEqual([entry.panel?.data, entry.structure, entry.values?.get('GER:total'), structure], [{ hour: 23 }, 'S23', 23, 1]);
  const version = entry.version;
  transport.reading(reading(23, 30), 'dated');
  assert.deepEqual([entry.through, entry.version, structure, live, landed()?.through], [30, version, 1, 2, 30],
    'a coverage patch dates the reading for its live readers without a new version or a structure change');
  transport.reading(reading(24), 'held');
  assert.deepEqual([entry.panel, entry.values, landed()?.panel.data, landed()?.values?.get('GER:total'), landed()?.structure], [undefined, undefined, { hour: 24 }, 24, 'S24'],
    'a held reading is kept apart with its values and structure, to be dimmed, and nothing is current');
  transport.reading(reading(24), 'current');
  assert.deepEqual(entry.panel?.data, { hour: 24 }, 'a resume shows what was held as current at once');
  transport.reading({ id: entry.id, hour: 25, through: 25, panel: null, answer: null, values: null }, 'empty');
  assert.deepEqual([entry.panel, landed(), entry.structure], [undefined, null, undefined], 'a discarding seek empties it and remembers that something had landed');
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

const GOLDEN = '464f4c3301000000d204000003000000000000000000000007000000b0040000d000000000000000010000000300000001000000050000006400000000000000650000000000000000000000000000000000000000000000000000000000000000000000000000004e61bc0000000000000000000000000000000000000000000001030000000000ffffffff02000000040000000000000000000000010000000300000000000000000000000300000006000000160000001900000032000000474552534f56534f5620686173206e6f2076616c75654954414954412069732068656c64206279206e6f20636f756e74727900000000000008000000d20400004000000000000000000000000000000000000000010000000000000027000000436f756e747269657320776173206e6f74207265636f72646564206174207468697320686f75720009000000d204000058000000000000000200000001000000020000000000000005000000000000004000000000000000feffffffffffffff09000000000000004c040000000000000000000000000000ffffffff00000000ffffffff000000000000000000000000';

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

interface Shown { failed?: true; content?: { structure: string; value: number | string | undefined }; through?: number; held?: boolean }
interface Segment { kind: 'discard' | 'carry' | 'failed'; serial: number; restart: number; hour: number; left: number; readings: Map<number, { hour: number; content: Content }[]>; completed: number[] }
interface Known { segments: Segment[]; drained: number; lead: number }
type Picture = { failed: true } | { content: Content; end: number } | null;

const pictureOf = (entry: FollowEntry): Shown => entry.status === 'failed' ? { failed: true }
  : entry.panel ? { content: { structure: entry.structure!, value: entry.values?.get(VALUE) }, through: entry.through, held: false }
  : entry.landed ? { content: { structure: entry.landed.structure!, value: entry.landed.values?.get(VALUE) }, through: entry.landed.through, held: true } : {};

function pictures(log: readonly Known[], at: number, window: number, id: number, hour: number, carry: number, applied?: number,
  index = log[at]!.segments.length - 1, memo = new Map<string, Picture[]>()): Picture[] {
  const key = `${at}:${window}:${hour}:${applied}:${index}`, known = memo.get(key);
  if (known) return known;
  const segment = log[at]!.segments[index]!, out: Picture[] = [];
  memo.set(key, out);
  for (let cut = Math.max(0, at - window); cut <= at; cut++) {
    const known = log[cut]!.segments[index];
    if (known?.serial !== segment.serial) continue;
    if (known.kind === 'failed') { out.push({ failed: true }); continue; }
    const readings = known.readings.get(id) ?? [], drained = applied ?? log[cut]!.drained, reach = drained + (applied === undefined ? log[cut]!.lead : 0);
    let found = -1;
    readings.forEach((reading, index) => { if (reading.hour <= hour && reading.hour <= reach) found = index; });
    if (found >= 0) {
      const confirmed = Math.max(-Infinity, ...known.completed.filter(through => through <= drained));
      out.push({ content: readings[found]!.content, end: Math.max(readings[found]!.hour, Math.min(readings[found + 1] ? readings[found + 1]!.hour - 1 : Infinity, confirmed)) });
      continue;
    }
    if (index === 0) { out.push(null); continue; }
    for (let before = Math.max(0, segment.restart - carry - 1); before <= segment.restart; before++) {
      if (log[before]!.segments[index - 1]?.serial !== log[segment.restart]!.segments[index - 1]!.serial) continue;
      const left = before === segment.restart ? log[before - 1]?.drained ?? 0 : undefined;
      for (const base of pictures(log, before, 0, id, segment.left, carry, left, index - 1, memo))
        out.push(base && 'failed' in base ? base : segment.kind === 'carry' ? base : null);
      if (segment.kind === 'carry') for (const kept of pictures(log, before, 0, id, segment.hour, carry, Math.max(segment.hour, log[before]!.drained), index - 1, memo))
        if (kept && 'content' in kept && kept.end >= segment.hour) out.push(kept);
    }
  }
  const unique = [...new Map(out.map(picture => [JSON.stringify(picture), picture])).values()];
  memo.set(key, unique);
  return unique;
}

function expected(log: readonly Known[], window: number, id: number, hour: number, paused: boolean): string[] {
  return [...new Set(pictures(log, log.length - 1, window, id, hour, window).map(picture => JSON.stringify(!picture ? {} : 'failed' in picture ? picture
    : { content: { structure: picture.content.structure, value: picture.content.value }, through: Math.min(hour, picture.end), held: paused && picture.end < hour })))];
}

async function randomRun(run: number, ops: number, random: (n: number) => number, mutate?: (p: Awaited<ReturnType<typeof pipeline>>) => void) {
  const ids = [1, 2, 3], WINDOW = 3, p = await pipeline(ids);
  mutate?.(p);
  const changes = new Map(ids.map(id => [id, new Map<number, Content>()]));
  let structures = 0, values = 0;
  for (const id of ids) for (let hour = random(10); hour < 6000; hour += 1 + random(30)) {
    const before = [...changes.get(id)!.values()].at(-1);
    changes.get(id)!.set(hour, { structure: !before || random(4) === 0 ? `S${++structures}` : before.structure, value: ++values });
  }
  const contentAt = (id: number, hour: number) => { let found: Content | undefined; for (const [at, content] of changes.get(id)!) if (at <= hour) found = content; return found; };
  let segments: Segment[] = [{ kind: 'carry', serial: 0, restart: 0, hour: 0, left: 0, readings: new Map(), completed: [] }];
  const log: Known[] = [];
  let read = -1, fresh = true, frame = 1, drained = 0, lead = 0, held = false;
  const snapshot = (): Known => ({ drained, lead, segments: segments.map(segment => ({ ...segment, completed: [...segment.completed], readings: new Map([...segment.readings].map(([id, list]) => [id, [...list]])) })) });
  let left = 0;
  const restart = (kind: Segment['kind'], hour: number) => {
    segments = [...segments, { kind, serial: segments.length, restart: frame, hour, left, readings: new Map(), completed: [] }];
    read = hour - 1; fresh = true; drained = hour;
  };
  const native = (to: number) => {
    const s = p.clock.getSnapshot(), segment = segments.at(-1)!;
    if (to <= read || segment.kind === 'failed') return;
    const batch: { id: number; hour: number; content: Content }[] = [];
    for (let hour = read + 1; hour <= to; hour++) for (const id of ids) {
      const content = contentAt(id, hour);
      if (!content || !(fresh && hour === read + 1) && !changes.get(id)!.has(hour)) continue;
      batch.push({ id, hour, content });
    }
    fresh = false; read = to;
    p.lane.land([panels(s.seekSequence, to, batch)]);
    for (const { id, hour, content } of batch) {
      let list = segment.readings.get(id);
      if (!list) segment.readings.set(id, list = []);
      list.push({ hour, content });
    }
    segment.completed.push(to);
  };
  const check = (where: string) => {
    const s = p.clock.getSnapshot();
    for (const id of ids) {
      const shown = JSON.stringify(pictureOf(p.entries.get(id)!)), allowed = expected(log, WINDOW, id, s.hour, !s.playing);
      if (!allowed.includes(shown))
        assert.fail(`run ${run} ${where} at hour ${s.hour}${s.playing ? '' : ' (paused)'}: subscription ${id} shows ${shown}; within ${WINDOW} frames a direct read gives one of ${allowed.join(' ')}`);
    }
  };
  const step = 24;
  log.push(snapshot());
  for (let op = 0; op < ops; op++, frame++) {
    const s = p.clock.getSnapshot(), kind = random(20), where = `op ${op} (${kind})`;
    left = s.hour;
    if (random(3)) native(Math.max(read, s.hour + random(72)));
    if (kind < 12) p.clock.frame(s.playing ? s.position + random(step + 1) : s.position);
    else if (kind < 14) { if (s.playing) { p.clock.pause(); restart('carry', p.clock.getSnapshot().hour); } else p.clock.resume(); }
    else if (kind === 14) { const to = s.hour + 1 + random(60); p.clock.seek(to); restart('carry', to); }
    else if (kind === 15 && s.hour > 0) { const to = Math.max(0, s.hour - 1 - random(60)); p.clock.seek(to); restart('discard', to); }
    else if (kind === 16 && !s.playing) { p.clock.step(1); restart('carry', p.clock.getSnapshot().hour); }
    else if (kind === 17 && !s.playing && s.hour > 0) { p.clock.step(-1); restart('discard', p.clock.getSnapshot().hour); }
    else if (kind === 18 && random(8) === 0) { p.lane.land([{ kind: 'failure', reason: 'stream closed' }]); restart('failed', s.hour); p.clock.frame(s.position); }
    else p.clock.frame(s.position);
    log.push(snapshot());
    check(`${where}, as the clock publishes`);
    const now = p.clock.getSnapshot();
    drained = Math.max(drained, now.hour + (now.playing ? random(step) : 0)); lead = now.playing ? 2 * step : 0;
    p.lane.frame(drained, lead);
    log[log.length - 1]!.drained = drained; log[log.length - 1]!.lead = lead;
    if (held || random(4)) { held = false; while (p.lane.deliver()); } else held = true;
    check(`${where}, after its messages`);
  }
}

test('every page frame, through seeks, pauses, steps and failures, shows a direct read of its own hour no more than three frames old', async () => {
  let seed = 0x2545f491;
  const random = (n: number) => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return (seed >>> 0) % n; };
  for (let run = 0; run < 300; run++) await randomRun(run, 120, random);
});

const settle = (p: Awaited<ReturnType<typeof pipeline>>, head: number, ahead = 48) => { p.lane.frame(head, ahead); while (p.lane.deliver()); };
const shown = (p: Awaited<ReturnType<typeof pipeline>>, id = 1) => pictureOf(p.entries.get(id)!);
const content = (structure: string, value: number) => ({ structure, value });

test('a reading the page still shows is kept until the page has moved past it, though the worker drained ahead', async () => {
  const p = await pipeline([1]);
  for (let day = 0; day <= 5; day++) p.lane.land([panels(0, day * 24 + 23, [{ id: 1, hour: day * 24, content: content('S', day) }])]);
  for (const hour of [23, 47, 71, 95, 119]) {
    p.clock.frame(hour - 1); settle(p, hour);
    p.clock.frame(hour); settle(p, hour + 1);
    assert.deepEqual(shown(p), { content: { structure: 'S', value: (hour - 23) / 24 }, through: hour, held: false },
      `at ${hour}, a worker one hour ahead has made the next day current, and the page still shows its own day`);
  }
});

test('a reading becomes current at the hour the clock published, whatever the worker or the wall clock reached', async () => {
  const p = await pipeline([1]);
  p.lane.land([panels(0, 40, [{ id: 1, hour: 0, content: content('S', 0) }, { id: 1, hour: 12, content: content('S', 12) }])]);
  p.clock.frame(0); settle(p, 30);
  assert.deepEqual(shown(p), { content: { structure: 'S', value: 0 }, through: 0, held: false }, 'the worker drained to 30, the page published 0');
  p.clock.frame(12);
  assert.deepEqual(shown(p)?.content, { structure: 'S', value: 12 }, 'the reading sent ahead shows when the clock publishes its hour, before any message');
});

test('a seek empties what it discards before any frame listener runs for the new position', async () => {
  const p = await pipeline([1]);
  p.lane.land([panels(0, 120, [{ id: 1, hour: 100, content: content('S', 100) }])]);
  p.clock.frame(100); settle(p, 100);
  const seen: Shown[] = [];
  p.clock.frames.add(() => seen.push(shown(p)));
  p.clock.seek(20);
  assert.deepEqual(seen, [{}], 'a frame listener of the seek to 20 sees nothing of hour 100');
  settle(p, 20, 0);
  assert.deepEqual(shown(p), {}, 'and nothing of it arrives later');
});

test('a reading whose structure is unchanged crosses without its panel, and the page keeps the panel it decoded with the new values', async () => {
  const p = await pipeline([1]);
  p.lane.land([panels(0, 10, [{ id: 1, hour: 0, content: content('S', 1) }, { id: 1, hour: 5, content: content('S', 2) }, { id: 1, hour: 8, content: content('T', 3) }])]);
  p.clock.frame(0); settle(p, 0);
  const first = p.entries.get(1)!.panel;
  p.clock.frame(5);
  const puts = p.lane.delivered.flat().flatMap(update => update.put ?? []);
  assert.deepEqual(puts.map(put => [put.due, put.kept ?? false, put.panel !== null, put.structure ?? null]), [[0, false, true, 'S'], [5, true, false, null], [8, false, true, 'T']]);
  assert.equal(p.entries.get(1)!.panel, first, 'the page keeps the panel object of the structure');
  assert.deepEqual(shown(p), { content: { structure: 'S', value: 2 }, through: 5, held: false }, 'and shows the new values');
  p.clock.frame(8);
  assert.deepEqual(shown(p).content, { structure: 'T', value: 3 });
});

test('a coverage-only confirmation moves the coverage of the reading the page holds, never resends it', () => {
  const followed = new Followed(() => {});
  followed.add(3, 'branch', () => 'S');
  followed.bind('branch', 9, 1);
  followed.land([panels(1, undefined, [{ id: 3, hour: 10, content: content('S', 1) }])]);
  followed.drain(10);
  assert.equal([...followed.updates(10)][0]?.put?.length, 1);
  followed.land([{ kind: 'panels', batch: { route: 9, clock: 0, seek: 1, completed: 14, panels: [{ id: 3, hour: 14, panel: null, stepped: true }] } }]);
  followed.drain(14);
  assert.deepEqual([...followed.updates(14)], [{ id: 3, epoch: 0, cover: [[10, 14]] }]);
});

test('a follow failure is terminal for its epoch on both sides and named on the page, until a later epoch brings data', async () => {
  const p = await pipeline([1]);
  p.lane.land([panels(0, 10, [{ id: 1, hour: 0, content: content('S', 1) }])]);
  p.clock.frame(5); settle(p, 5);
  p.lane.land([{ kind: 'failure', reason: 'stream closed' }]);
  assert.deepEqual([p.entries.get(1)!.status, p.entries.get(1)!.reason], ['failed', 'stream closed']);
  p.lane.land([panels(0, 20, [{ id: 1, hour: 6, content: content('S', 6) }])]);
  p.clock.frame(8); settle(p, 8);
  assert.deepEqual(shown(p), { failed: true }, 'a panel after the failure is not taken by the worker nor the page');
  p.clock.seek(30);
  p.lane.land([panels(1, 30, [{ id: 1, hour: 30, content: content('S', 30) }])]);
  settle(p, 30, 0);
  assert.deepEqual(shown(p), { content: { structure: 'S', value: 30 }, through: 30, held: false }, 'a later epoch with data recovers');
});

test('pausing keeps the picture held, and resuming shows it again with no new reading', async () => {
  const p = await pipeline([1]);
  p.lane.land([panels(0, 23, [{ id: 1, hour: 23, content: content('S', 23) }])]);
  p.clock.frame(23); settle(p, 23);
  p.clock.frame(24.5);
  p.clock.pause();
  assert.deepEqual(shown(p), { content: { structure: 'S', value: 23 }, through: 23, held: true }, 'paused at 24 with a reading of 23, it is held, not blank');
  settle(p, 24, 0);
  assert.deepEqual(shown(p), { content: { structure: 'S', value: 23 }, through: 23, held: true }, 'and stays held while nothing covers 24');
  p.clock.resume();
  assert.deepEqual(shown(p), { content: { structure: 'S', value: 23 }, through: 23, held: false }, 'resuming shows it as current at once');
});
