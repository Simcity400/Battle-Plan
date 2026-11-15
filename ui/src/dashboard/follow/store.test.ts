import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, cellState, cellValue, parseFollowed, rowHour, rowKeys, rowName, rowReason } from './message.ts';
import type { FollowedAnswer } from './message.ts';
import { fakeTransport, manualFrames, message } from './messages.fixture.ts';
import { BarCell, TextCell, formatAmountFast, paintArmy, paintMeasure, paintStability } from './painters.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { FollowStore } from './store.ts';
import type { Availability } from './store.ts';
import { followStore, followTransport } from './connection.ts';
import { followsHead } from './useFollow.ts';
import { panelReply } from './panel.ts';

const one = (key: bigint, value: bigint) => ({ keys: [key], columns: 1, values: [value] });

/** A text cell over plain objects, as an element's text node and title. */
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
  assert.equal(frames.run(), 1, 'a new painter is due once');
  assert.deepEqual(painted, [0]);
  transport.deliver(message(24, [{ id: 1, answer: one(100n, 5n) }]));
  transport.deliver(message(48, [{ id: 1, answer: one(100n, 6n) }]));
  transport.deliver(message(72, [{ id: 1, answer: one(100n, 7n) }]));
  assert.equal(frames.pending, 1, 'three messages, one frame');
  frames.run();
  assert.deepEqual(painted, [0, 72], 'painted once, with the newest');
  frames.run();
  transport.deliver(message(96, [{ id: 2, answer: one(1n, 1n) }], false, 1));
  assert.equal(frames.pending, 0, 'nothing due when its entry did not move');
  assert.deepEqual(painted, [0, 72]);
});

test('an answer not resent is current through the newest message of its timeline, and its painter hears it', () => {
  const transport = fakeTransport();
  const frames = manualFrames();
  const store = new FollowStore(transport, frames.schedule);
  const moved = store.acquire({ product: 'series-state', params: { branchId: '7', tags: ['GER'] } }).entry;
  const still = store.acquire({ product: 'series-state', params: { branchId: '7', tags: ['ITA'] } }).entry;
  const shown: number[] = [];
  store.painter(still, entry => shown.push(entry.through));
  frames.run();
  transport.deliver(message(24, [{ id: moved.id, answer: one(1n, 5n) }, { id: still.id, answer: one(2n, 9n) }]));
  transport.deliver(message(48, [{ id: moved.id, hour: 40, answer: one(1n, 6n) }]));
  assert.deepEqual([moved.hour, moved.through, still.hour, still.through], [40, 48, 24, 48], 'the unchanged answer holds at the message\'s hour');
  frames.run();
  assert.deepEqual(shown, [0, 48], 'painted once, at the newest hour it holds through');
  transport.deliver(message(72, [], true));
  assert.equal(still.through, 72, 'a settled message with no answer still moves every query of the one timeline followed');
  const other = store.acquire({ panel: 'roster', branchId: '9' }).entry;
  const elsewhere = store.acquire({ product: 'series-state', params: { branchId: '9', tags: ['SOV'] } }).entry;
  transport.deliver(message(96, [{ id: moved.id, answer: one(1n, 7n) }]));
  assert.deepEqual([still.through, elsewhere.through, other.through], [96, 0, 0], 'another timeline\'s queries and panels hold their own hours');
  transport.deliver(message(100, [{ id: elsewhere.id, answer: one(3n, 1n) }], false, 1));
  transport.deliver(message(120, [], false, 1));
  assert.deepEqual([still.through, elsewhere.through], [96, 120], 'a message naming none moves the queries of its own playhead alone');
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
  // Their time is measured in a browser's frames (`follow.dom.test.ts`).
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
  assert.equal(store.counts.messages, 1);
  assert.equal(frames.pending, 1, 'one frame for every entry');
  for (let round = 0; round < 3; round++) {
    transport.deliver(message(100 + round, Array.from({ length: 500 }, (_, at) => ({ id: at + 1, answer: one(BigInt(at), BigInt(at + round) * 100_000n) }))));
    assert.equal(frames.run(), 1);
  }
  assert.equal(store.counts.messages, 4);
  assert.equal(store.counts.paints, 500 * 4, 'every painter once a frame: its first, then four messages in three frames');
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

test('a reset clears the rows a log held', () => {
  const transport = fakeTransport();
  const frames = manualFrames();
  const store = new FollowStore(transport, frames.schedule);
  const handle = store.acquire({ log: 1 });
  transport.deliver(message(30, [{ id: 1, answer: { keys: [5n], columns: 1, values: [1n], hours: [25] } }]));
  assert.equal(handle.entry.answer?.kind, 'log');
  transport.deliver(message(900, [], true));
  assert.equal(handle.entry.answer, undefined, 'rows from before the seek are gone');
});

test("a store hears only its own follows: another recording's messages move none of its hours and clear none of its logs", () => {
  const channel = fakeTransport(), other = fakeTransport();
  const deliver = (bytes: Uint8Array) => { channel.deliver(bytes); other.deliver(bytes); };
  const ids = (() => { let next = 1; return () => next++; })();
  const mine = new FollowStore(channel, manualFrames().schedule, ids), theirs = new FollowStore(other, manualFrames().schedule, ids);
  const log = mine.acquire({ log: 1, branchId: '1' }).entry, series = mine.acquire({ product: 'series-state', params: { branchId: '1' } }).entry;
  const elsewhere = theirs.acquire({ log: 1, branchId: '1' }).entry;
  deliver(message(30, [{ id: log.id, answer: { keys: [5n], columns: 1, values: [1n], hours: [25] } }, { id: series.id, answer: one(1n, 5n) }], false, 4));
  deliver(message(70, [{ id: elsewhere.id, answer: { keys: [6n], columns: 1, values: [2n], hours: [60] } }], false, 9));
  deliver(message(900, [], true, 9));
  assert.equal(log.answer?.kind, 'log', "another recording's seek leaves this log's rows");
  assert.equal(elsewhere.answer, undefined, 'its own seek clears its own');
  deliver(message(80, [], false, 9));
  assert.deepEqual([log.through, series.through], [30, 30], "another recording's hours move none of these");
  deliver(message(48, [], false, 4));
  assert.deepEqual([log.through, series.through, elsewhere.through], [48, 48, 80], 'a message of their own follow moves them');
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

/** A message the service wrote (`crates/dashboard-follow/src/tests.rs`, `golden`): the same bytes. */
const GOLDEN = '464f4c3101000000d20400000300000007000000b0040000c800000000000000010000000300000001000000050000006400000000000000650000000000000000000000000000000000000000000000000000000000000000000000000000004e61bc0000000000000000000000000000000000000000000001030000000000ffffffff020000000400000000000000000000000100000003000000000000000300000047455203000000534f5610000000534f5620686173206e6f2076616c756503000000495441190000004954412069732068656c64206279206e6f20636f756e747279000008000000d204000040000000000000000000000000000000000000000100000027000000436f756e747269657320776173206e6f74207265636f72646564206174207468697320686f7572000000000009000000d204000050000000000000000200000001000000020000000000000005000000000000004000000000000000feffffffffffffff09000000000000004c040000000000000000000000000000ffffffff00000000ffffffff00000000';

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

test('a notice that pushed answers stopped makes every query unavailable by name, now and later', async () => {
  const transport = fakeTransport();
  const store = new FollowStore(transport, manualFrames().schedule);
  const power = store.acquire({ q: 'power' });
  let told = 0;
  store.watch(power.entry, () => told++);
  assert.equal(power.entry.status, 'followed');
  const reason = 'Pushed answers stopped: their pipe could not be read (broken)';
  transport.deliver(new TextEncoder().encode(`FEN1${reason}`));
  assert.deepEqual([power.entry.status, power.entry.reason, told], ['unavailable', reason, 1], 'the widget is told, so it asks as before');
  const stability = store.acquire({ q: 'stability' });
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual([stability.entry.status, stability.entry.reason], ['unavailable', reason]);
  assert.equal(transport.sent.filter(sent => sent.startsWith('follow ')).length, 1, 'nothing more is followed');
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

test('a query the service does not follow is unavailable with its reason, and one it follows is followed', async () => {
  const told = new Map<number, Availability>([[1, { available: false, reason: 'CountryStability is not in this playhead\'s files' }], [2, { available: true }]]);
  const watched: string[] = [];
  const store = new FollowStore({ follow: id => Promise.resolve(told.get(id)!), unfollow: () => {}, listen: () => () => {} }, manualFrames().schedule);
  const stability = store.acquire({ q: 'stability' }), power = store.acquire({ q: 'power' });
  assert.equal(stability.entry.status, 'pending');
  store.watch(stability.entry, () => watched.push(stability.entry.status));
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(stability.entry.status, 'unavailable');
  assert.equal(stability.entry.reason, 'CountryStability is not in this playhead\'s files');
  assert.deepEqual(watched, ['unavailable'], 'the widget is told, so it keeps asking as before');
  assert.equal(power.entry.status, 'followed');
  assert.equal(followsHead({ status: stability.entry.status, structure: '', entry: stability.entry, hidden: false }), false);
  assert.equal(followsHead({ status: power.entry.status, structure: '', entry: power.entry, hidden: false }), true);
  const bridge = followTransport({ available: true } as never, { id: 's', path: 'p', revision: 'r' } as never);
  assert.deepEqual(await bridge.follow(3, { product: 'series-state', params: { branchId: '1' } }),
    { available: false, reason: 'This connection has no channel for pushed answers' }, 'a client with no channel follows nothing');
});

test('a panel lands in its entry, tells its watchers only when its reply differs, and a malformed one is refused', () => {
  const transport = fakeTransport();
  const store = new FollowStore(transport, manualFrames().schedule);
  const { entry } = store.acquire({ panel: 'template-roster', branchId: '1', tag: 'GER' });
  let told = 0;
  store.watch(entry, () => told++);
  const panel = (hour: number, sections: string[], refused: string | null = null) =>
    new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: entry.id, hour, data: refused === null ? { sections, warnings: ['w'] } : null, refused }, { id: 999, hour, data: {}, refused: null }] })}`);
  transport.deliver(panel(10, ['Infantry division']));
  assert.deepEqual(entry.panel?.data, { sections: ['Infantry division'], warnings: ['w'] });
  assert.equal(entry.answer, undefined, 'a panel is no followed answer');
  const version = entry.version;
  transport.deliver(panel(11, ['Infantry division']));
  assert.equal(entry.version, version, 'the same text moves nothing');
  assert.equal(entry.hour, 11);
  transport.deliver(panel(12, ['Armoured division']));
  transport.deliver(panel(13, [], 'Template roster country is unavailable at this hour'));
  assert.equal(told, 3, 'told for the first text and each that differed');
  assert.equal(entry.panel?.refused, 'Template roster country is unavailable at this hour');
  assert.throws(() => transport.deliver(new TextEncoder().encode('PNL1{"panels":[{"id":1}]}')), /malformed/);
});

test('a panel carrying a reply lands as data, is painted and told only when the reply differs, and validates once', () => {
  const transport = fakeTransport();
  const frames = manualFrames();
  const store = new FollowStore(transport, frames.schedule);
  const { entry } = store.acquire({ panel: 'naval-yards', tag: 'ENG', branchId: '1' });
  let told = 0, validated = 0;
  const painted: number[] = [];
  store.watch(entry, () => told++);
  store.painter(entry, held => painted.push(held.hour));
  frames.run();
  const push = (hour: number, data: unknown, refused: string | null = null) =>
    transport.deliver(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: entry.id, hour, data, refused }] })}`));
  const validate = (data: unknown) => { validated++; return data as { rows: { yard: string; ships: number }[] }; };
  push(10, { rows: [{ yard: 'Portsmouth', ships: 3 }] });
  assert.deepEqual(entry.panel?.data, { rows: [{ yard: 'Portsmouth', ships: 3 }] });
  frames.run();
  push(11, { rows: [{ yard: 'Portsmouth', ships: 3 }] });
  assert.equal(frames.pending, 0, 'an equal reply is not painted again');
  push(12, { rows: [{ yard: 'Portsmouth', ships: 4 }] });
  frames.run();
  assert.deepEqual(painted, [0, 10, 12]);
  assert.equal(told, 2);
  assert.deepEqual(panelReply(entry.panel, validate).data, { rows: [{ yard: 'Portsmouth', ships: 4 }] });
  panelReply(entry.panel, validate);
  assert.equal(validated, 1, 'one push is validated once however often it is read');
  assert.match(panelReply(entry.panel, () => { throw new Error('The yards reply is malformed.'); }).error?.message ?? '', /yards reply/,
    'a reply read through another validation is that validation');
  push(13, null, 'No country holds ENG at this hour');
  assert.equal(panelReply(entry.panel, validate).error?.message, 'No country holds ENG at this hour');
  assert.deepEqual(panelReply(undefined, validate), {}, 'nothing before the first push');
});

test('a panel naming its history beside its params anchors the clock for that history', async () => {
  const asked: { product: string; params: Record<string, unknown> }[] = [];
  const client = { product: (request: { product: string; params: Record<string, unknown> }) => {
    asked.push(request);
    return Promise.resolve({ ...request, data: request.product === 'follow' ? { available: true } : {} });
  }, onFollowMessages: () => Promise.resolve(() => {}) };
  const clock = { getSnapshot: () => ({ playing: true, hoursPerSecond: 1440, seekSequence: 0, position: 100 }), subscribe: () => () => {} };
  const transport = followTransport(client as never, { id: 's', revision: 'r' } as never, clock as never);
  assert.deepEqual(await transport.follow(1, { panel: 'naval-yards', tag: 'ENG', branchId: 'b' }), { available: true });
  assert.deepEqual(asked.map(request => [request.product, request.params.branchId]), [['follow', 'b'], ['follow-clock', 'b']]);
});

test('a track aligned at its anchor follows the game hour it shows, and each clock mapping has its own store', async () => {
  const asked: { product: string; params: Record<string, unknown> }[] = [];
  const client = { product: (request: { product: string; params: Record<string, unknown> }) => {
    asked.push(request);
    return Promise.resolve({ ...request, data: { available: true } });
  }, onFollowMessages: () => Promise.resolve(() => {}) };
  const clock = { getSnapshot: () => ({ playing: false, hoursPerSecond: 24, seekSequence: 3, position: 10.5 }), subscribe: () => () => {} };
  const source = { id: 's', revision: 'r' } as never;
  const transport = followTransport(client as never, source, clock as never, undefined, 1000);
  await transport.follow(1, { product: 'series-state', params: { branchId: 'b' } });
  assert.deepEqual(asked.map(({ product, params }) => [product, params.clock, params.hour]), [['follow', 1000, undefined], ['follow-clock', 1000, 1010.5]],
    'the hour the track shows (its anchor on), and the mapping the host keys its playhead by');
  assert.notEqual(followStore(client as never, source, clock as never, 1000), followStore(client as never, source, clock as never, 0));
  assert.equal(followStore(client as never, source, clock as never, 1000), followStore(client as never, source, clock as never, 1000));
});

test('paused, an hour stepped moves the followed playhead to it; a pause alone seeks nothing', async () => {
  const asked: { product: string; params: Record<string, unknown> }[] = [];
  const client = { product: (request: { product: string; params: Record<string, unknown> }) => {
    asked.push(request);
    return Promise.resolve({ ...request, data: { available: true } });
  }, onFollowMessages: () => Promise.resolve(() => {}) };
  let snapshot = { playing: true, hoursPerSecond: 24, seekSequence: 1, position: 40.5 };
  const listeners = new Set<() => void>();
  const clock = { getSnapshot: () => snapshot, subscribe: (listener: () => void) => { listeners.add(listener); return () => listeners.delete(listener); } };
  const move = (next: Partial<typeof snapshot>) => { snapshot = { ...snapshot, ...next }; for (const listener of listeners) listener(); };
  const transport = followTransport(client as never, { id: 's', revision: 'r' } as never, clock as never, undefined, 100);
  await transport.follow(1, { product: 'series-state', params: { branchId: 'b' } });
  const anchors = () => asked.filter(({ product }) => product === 'follow-clock').map(({ params }) => [params.hour, params.playing, params.sought]);
  move({ playing: false, position: 52.25 });
  move({ position: 53.25 });
  move({ position: 53.5 });
  move({ position: 52.25 });
  assert.deepEqual(anchors(), [[140.5, true, true], [152.25, false, false], [153.25, false, true], [152.25, false, true]],
    'Next hour and Previous hour each move it; within the hour, or a pause, nothing is sought');
});
