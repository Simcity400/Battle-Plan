import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { CELLS, NOT_READ, ZERO_DIVISOR, followedCells, liveSection, repliedCells } from './liveCells.ts';
import type { LiveSection } from './liveCells.ts';
import { fakeTransport, manualFrames, message } from './messages.fixture.ts';
import { FollowStore } from './store.ts';

/** Two countries' power (group 0: Countries, column 2, keys 100 and 101) and one's coal (group 1: two columns of key 7). */
const section = (groups: LiveSection['groups']): LiveSection => ({
  groups,
  rows: ['100', '101'],
  kinds: [{ id: 'power', op: 'value', scale: 0.001 }, { id: 'coal', op: 'sum', scale: 1 }, { id: 'net', op: 'difference', scale: 1 }, { id: 'share', op: 'ratio', scale: 100 }],
  cells: [
    0, 0, 1, 0, 0, 0,
    1, 0, 1, 0, 1, 0,
    0, 1, 2, 1, 0, 0, 1, 0, 1,
    0, 2, 2, 1, 0, 0, 1, 0, 1,
    0, 3, 2, 0, 0, 0, 0, 1, 0,
  ],
});
const countries = { table: 2, columns: [2], keys: ['100', '101'] };
const coal = { table: 52, columns: [4, 5], keys: ['7'] };

test('a request\'s reply gives each cell its value by its formula, and a missing one its reason, never zero', () => {
  const live = repliedCells({ live: section([{ ...countries, values: [5000, 'No live row has this key'] }, { ...coal, values: [900, -100] }]) });
  const values = live.read();
  assert.equal(values.get(liveKey('100', 'power')), 5);
  assert.equal(values.get(liveKey('101', 'power')), undefined);
  assert.equal(values.missing(liveKey('101', 'power')), 'No live row has this key');
  assert.equal(values.get(liveKey('100', 'coal')), 800, 'a sum');
  assert.equal(values.get(liveKey('100', 'net')), 1000, 'a difference');
  assert.equal(values.get(liveKey('100', 'share')), undefined, 'a ratio over a missing divisor');
  assert.equal(values.missing(liveKey('100', 'share')), 'No live row has this key');
  assert.equal(values.missing(liveKey('100', 'nothing')), 'Not a live cell of this reply');
  const zero = repliedCells({ live: section([{ ...countries, values: [5000, 0] }, { ...coal, values: [1, 1] }]) }).read();
  assert.equal(zero.missing(liveKey('100', 'share')), ZERO_DIVISOR);
  assert.equal(repliedCells({ live: section([{ ...countries, values: [5000, 0] }, { ...coal, unavailable: 'CountryResources is not in this playhead\'s files' }]) }).read()
    .missing(liveKey('100', 'coal')), 'CountryResources is not in this playhead\'s files');
  assert.equal(liveSection({ warnings: [] }), null, 'a reply naming no cells');
  assert.throws(() => liveSection({ live: { ...section([{ ...countries, values: [1, 2] }]), cells: [0, 0, 1, 5, 0, 0] } }), /source no group holds/);
  assert.throws(() => liveSection({ live: section([{ ...countries, follow: 7 }, { ...coal, values: [1, 1] }]) }), /malformed/, 'a UI id is never a group\'s');
});

test('a reply or a panel naming no cells, or failing, reads the same values every time, as unmoved cells do', () => {
  const none = repliedCells({ warnings: [] });
  assert.equal(none.read(), none.read(), 'a reply naming no cells');
  const quiet = { group: () => undefined, subscribe: () => () => {} };
  const empty = followedCells({ ...quiet, section: () => null });
  assert.equal(empty.read(), empty.read(), 'a panel with no section yet');
  const failing = followedCells({ ...quiet, section: () => { throw new Error('The panel is malformed'); } });
  assert.equal(failing.read(), failing.read(), 'a panel whose section fails');
  assert.equal(failing.read().missing(liveKey('100', 'power')), 'The panel is malformed');
});

/** A panel message: panel `id`'s reply naming its power cells followed under `follow`. */
function panel(id: number, hour: number, rows: string[], follow: number, made?: number): Uint8Array {
  const data = { rows, ...(made === undefined ? {} : { made }), live: { groups: [{ table: 2, columns: [2], keys: rows, follow }], rows, kinds: [{ id: 'power', op: 'value', scale: 1 }],
    cells: rows.flatMap((_, row) => [row, 0, 1, 0, row, 0]) } };
  return new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id, hour, data, refused: null }] })}`);
}

test('a panel\'s cells land from their followed answers: whole, then the rows changed, then not recorded; a structure change replaces them', () => {
  const transport = fakeTransport();
  const store = new FollowStore(transport, manualFrames().schedule);
  const handle = store.acquire({ panel: 'test-power', branchId: '1', tag: 'GER' });
  const first = CELLS + 10, second = CELLS + 11;
  const live = followedCells({ section: () => liveSection(handle.entry.panel?.data), group: id => store.cellGroup(id), subscribe: changed => store.watchLive(handle.entry, changed) });
  let woken = 0;
  live.subscribe(() => woken++);
  // The cells' first answer may come before the reply naming them.
  transport.deliver(message(100, [{ id: first, answer: { keys: [100n, 101n], columns: 1, values: [5n, 6n] } }]));
  transport.deliver(panel(handle.entry.id, 100, ['100', '101'], first));
  assert.equal(live.read().get(liveKey('100', 'power')), 5);
  const read = live.read();
  assert.equal(live.read(), read, 'the same values until a group moves');
  assert.equal(store.liveThrough(handle.entry), 100, 'the hour its shown cells are of');
  transport.deliver(message(101, [{ id: first, answer: { keys: [101n], columns: 1, values: [7n], changed: true } }]));
  assert.equal(store.liveThrough(handle.entry), 101);
  assert.notEqual(live.read(), read);
  assert.deepEqual([live.read().get(liveKey('100', 'power')), live.read().get(liveKey('101', 'power'))], [5, 7], 'a changed row laid over the rows held');
  transport.deliver(message(102, [{ id: first, answer: { notRecorded: 'Countries was not recorded at this hour' } }]));
  assert.equal(live.read().get(liveKey('100', 'power')), undefined);
  assert.equal(live.read().missing(liveKey('100', 'power')), 'Countries was not recorded at this hour', 'missing, never the value before');
  assert.equal(woken, 3, 'woken by the reply naming the cells and by each answer of them after it');
  // A new structure names new cells; a late answer of the old group is dropped.
  transport.deliver(panel(handle.entry.id, 110, ['100'], second));
  assert.equal(live.read().missing(liveKey('100', 'power')), NOT_READ);
  assert.equal(store.liveThrough(handle.entry), null, 'no hour is shown before the new cells answer');
  transport.deliver(message(110, [{ id: first, answer: { keys: [100n, 101n], columns: 1, values: [9n, 9n] } }]));
  assert.equal(store.cellGroup(first), undefined, 'a group let go stays gone');
  transport.deliver(message(111, [{ id: second, answer: { keys: [100n], columns: 1, values: [42n], states: [0] } }]));
  assert.equal(live.read().get(liveKey('100', 'power')), 42);
  assert.equal(live.read().get(liveKey('101', 'power')), undefined, 'a row the structure no longer shows');
  handle.release();
  assert.equal(store.cellGroup(second), undefined, 'a released panel takes its cells');
});

test('cells answered before their panel keep their playhead: a later step that resends none of them still holds them through its hour', () => {
  const transport = fakeTransport();
  const store = new FollowStore(transport, manualFrames().schedule);
  const handle = store.acquire({ panel: 'test-power', branchId: '1', tag: 'GER' });
  const group = CELLS + 30;
  transport.deliver(message(100, [{ id: group, answer: { keys: [100n, 101n], columns: 1, values: [5n, 6n] } }], false, 3));
  transport.deliver(panel(handle.entry.id, 100, ['100', '101'], group));
  assert.equal(store.liveThrough(handle.entry), 100);
  transport.deliver(message(101, [], false, 3));
  assert.equal(store.liveThrough(handle.entry), 101, 'a step of its playhead with its cells unchanged');
  transport.deliver(message(102, [], false, 4));
  assert.equal(store.liveThrough(handle.entry), 101, 'another playhead\'s step is not its own');
});

test('a page of panels alone: each step its cells hold through moves their hour, and a panel pushed again naming the same group keeps its values', () => {
  const transport = fakeTransport();
  const store = new FollowStore(transport, manualFrames().schedule);
  const handle = store.acquire({ panel: 'test-power', branchId: '1', tag: 'GER' });
  const group = CELLS + 20;
  const live = followedCells({ section: () => liveSection(handle.entry.panel?.data), group: id => store.cellGroup(id), subscribe: changed => store.watchLive(handle.entry, changed) });
  let woken = 0;
  live.subscribe(() => woken++);
  transport.deliver(panel(handle.entry.id, 100, ['100', '101'], group));
  transport.deliver(message(100, [{ id: group, answer: { keys: [100n, 101n], columns: 1, values: [5n, 6n] } }]));
  const before = woken;
  transport.deliver(message(130, []));
  assert.equal(store.liveThrough(handle.entry), 130, 'a step with nothing changed still holds the cells through its hour');
  assert.ok(woken > before, 'and wakes the painter that reports it');
  transport.deliver(panel(handle.entry.id, 140, ['100', '101'], group, 140));
  assert.equal(handle.entry.panel?.data && (handle.entry.panel.data as { made?: number }).made, 140, 'the new reply landed');
  assert.equal(live.read().get(liveKey('100', 'power')), 5, 'the same group under the same id: its values stay, never "Not read yet"');
  assert.equal(store.liveThrough(handle.entry), 130);
});
