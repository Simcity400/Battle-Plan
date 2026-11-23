import assert from 'node:assert/strict';
import { test } from 'node:test';
import { changeWindow, contentVersion, Facts, joinComponents, samePicture, type FollowMessage, type PanelRecord, type Picture } from './facts.ts';

type V = unknown;
const R = 9;
const A = 0x80000001, B = 0x80000002, C = 0x80000003;
const data = (id: number, hour: number, extra: Partial<Extract<PanelRecord<V>, { kind: 'data' }>> = {}): PanelRecord<V> =>
  ({ kind: 'data', id, hour, value: `${id}@${hour}`, ...extra });
const stands = (id: number, hour: number, stepped = false): PanelRecord<V> => ({ kind: 'stands', id, hour, stepped });
const waiting = (id: number, hour: number): PanelRecord<V> => ({ kind: 'waiting', id, hour });
const msg = (hour: number, answers: FollowMessage<V>['answers'] = [], extra: Partial<FollowMessage<V>> = {}): FollowMessage<V> => ({ hour, answers, ...extra });
const hc = (p: Picture<V> | null) => p && [p.h, p.c];
const facts = (...subs: [number, 'panel' | 'followed'][]) => { const f = new Facts<V>(); for (const [id, channel] of subs) f.follow(id, R, channel); return f; };

test('COMPLETED(34) after three seeds at 10, two stepped, confirms 34, 34, 10', () => {
  const f = facts([1, 'panel'], [2, 'panel'], [3, 'panel']);
  f.panels(R, 0, [data(1, 10, { stepped: true }), data(2, 10, { stepped: true }), data(3, 10)]);
  f.panels(R, 0, [], 34);
  assert.deepEqual([1, 2, 3].map(id => f.select(id, 40)!.c), [34, 34, 10]);
  assert.deepEqual([1, 2, 3].map(id => f.select(id, 33)!.h), [10, 10, 10]);
});

test('10/20/30 selects 20 at 24 and never extends 10 by inference', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10), data(1, 20), data(1, 30)]);
  assert.deepEqual(hc(f.select(1, 24)), [20, 20]);
  assert.equal(f.select(1, 15)!.c, 10);
  const g = facts([1, 'panel']);
  g.panels(R, 0, [data(1, 10), data(1, 20), stands(1, 26)]);
  assert.deepEqual(hc(g.select(1, 24)), [20, 26]);
});

test('WAITING closes the latest fact; COMPLETED does not cross it; STANDS reopens', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { stepped: true })], 20);
  f.panels(R, 0, [waiting(1, 21)], 30);
  assert.equal(f.select(1, 30)!.c, 20);
  f.panels(R, 0, [stands(1, 32)], 33);
  assert.equal(f.select(1, 33)!.c, 33);
});

test('a later record at the same hour replaces the earlier one, with a new content version', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10)]);
  const before = contentVersion(f.select(1, 10)!);
  f.panels(R, 0, [data(1, 10, { value: 'corrected' })]);
  assert.equal(f.select(1, 10)!.value, 'corrected');
  assert.notEqual(contentVersion(f.select(1, 10)!), before);
});

test('a per-id limit limits only its own reading', () => {
  const f = facts([1, 'followed'], [2, 'followed']);
  f.followed(R, 0, msg(10, [{ id: 1, hour: 10, value: 'log' }, { id: 2, hour: 10, value: 'country' }]));
  f.followed(R, 0, msg(40, [], { limited: new Map([[1, 10]]) }));
  assert.deepEqual([f.select(1, 40)!.c, f.select(2, 40)!.c], [10, 40]);
});

test('a same-seek native reset replaces an earlier hour and raises the reset generation', () => {
  const f = facts([1, 'followed']);
  f.followed(R, 0, msg(30, [{ id: 1, hour: 30, value: 'old' }]));
  assert.equal(f.generation(R).reset, 0);
  f.followed(R, 0, msg(20, [{ id: 1, hour: 20, value: 'reset' }], { reset: true }));
  assert.equal(f.select(1, 30)!.value, 'reset');
  assert.deepEqual(f.generation(R), { seek: 0, reset: 1 });
});

test('older seeks are dropped; a rewind never shows a later hour', () => {
  const f = facts([1, 'followed']);
  f.followed(R, 2, msg(100, [{ id: 1, hour: 95, value: 'late' }]));
  assert.equal(f.select(1, 100), null);
  f.seek(R, 2, 100);
  assert.equal(f.select(1, 100)!.h, 95);
  f.seek(R, 3, 20);
  f.followed(R, 2, msg(100, [{ id: 1, hour: 99, value: 'stale' }]));
  assert.equal(f.select(1, 20), null, '"Reading hour 20", never the hour-95 picture');
  f.followed(R, 3, msg(20, [{ id: 1, hour: 18, value: 'seed' }]));
  assert.equal(f.select(1, 20)!.value, 'seed');
});

test('cells join only where confirmed intervals intersect; the last complete join stays while its replacement is incomplete', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 12, { stepped: true, cells: [{ id: A, seedHour: 10, values: [1] }, { id: B, seedHour: 12, values: [2] }] })], 40);
  assert.equal(f.select(1, 12), null);
  f.followed(R, 0, msg(22, [{ id: A, hour: 10, value: 'a' }]));
  assert.deepEqual(hc(f.select(1, 12)), [12, 12]);
  f.followed(R, 0, msg(24, [{ id: B, hour: 12, value: 'b' }]));
  assert.deepEqual(hc(f.select(1, 20)), [12, 24]);
  f.panels(R, 0, [data(1, 25, { cells: [{ id: A, seedHour: 25, values: [1] }, { id: C }] })]);
  assert.deepEqual(hc(f.select(1, 26)), [12, 24], 'C unanswered: the last complete join stays');
  f.followed(R, 0, msg(27, [{ id: C, hour: 25, value: 'c' }]));
  assert.deepEqual(hc(f.select(1, 26)), [25, 25]);
  assert.deepEqual(f.unfollow(1).sort(), [A, B, C].sort(), 'cell ids released on last use');
});

test('review 2: an unstepped seed joined to the stepped delivery by STANDS is covered by COMPLETED', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10)]);
  f.panels(R, 0, [stands(1, 34, true)], 34);
  f.panels(R, 0, [], 58);
  assert.deepEqual(hc(f.select(1, 60)), [10, 58]);
});

test('review 2: WAITING stands the product through the hour before it waited, and later steps never cross it', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { stepped: true })]);
  f.panels(R, 0, [], 22);
  f.panels(R, 0, [waiting(1, 34)], 34);
  f.panels(R, 0, [], 58);
  assert.deepEqual(hc(f.select(1, 58)), [10, 33]);
});

test('review 2: the join is a search, not greedy: a later fact of cell A does not hide the complete join 10-18', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { cells: [{ id: A }, { id: B }] }), stands(1, 30)]);
  f.followed(R, 0, msg(18, [{ id: A, hour: 10, value: 'a10' }, { id: B, hour: 10, value: 'b10' }]));
  f.followed(R, 0, msg(20, [{ id: A, hour: 20, value: 'a20' }], { limited: new Map([[B, 18]]) }));
  const p = f.select(1, 30)!;
  assert.deepEqual(hc(p), [10, 18]);
  assert.deepEqual(p.parts.slice(1).map(x => x.value), ['a10', 'b10']);
});

test('review 2: a selection is not "shown" until the page says it installed it; pruning keeps the installed picture', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10), data(1, 20), data(1, 30)]);
  const installed = f.select(1, 10);
  assert.equal(f.select(1, 30)!.h, 30, 'a speculative selection at 30 changes nothing');
  f.prune(1, 30, [installed]);
  assert.equal(f.select(1, 10)!.h, 10, 'the installed picture at 10 is still there to repaint and inspect');
  f.prune(1, 30, [f.select(1, 30)]);
  assert.equal(f.select(1, 10), null, 'released once the page installed 30');
});

test('review 2: a cell group is released when the last retained fact naming it goes, not at unmount', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { cells: [{ id: A, seedHour: 10, values: 1 }, { id: B, seedHour: 10, values: 1 }] }),
    data(1, 20, { cells: [{ id: A, seedHour: 20, values: 1 }, { id: C, seedHour: 20, values: 1 }] })]);
  f.prune(1, 20, [f.select(1, 20)]);
  assert.deepEqual(f.drain(), [], 'the picture of 10 is what selection falls back to if 20 is corrected incomplete');
  f.panels(R, 0, [data(1, 30, { cells: [{ id: A, seedHour: 30, values: 1 }] })]);
  f.prune(1, 30, [f.select(1, 30)]);
  assert.deepEqual(f.drain(), [B]);
  assert.deepEqual(f.unfollow(1).sort(), [A, C].sort());
});

test('a picture carries the window of targets it is the selection for', () => {
  const f = facts([1, 'followed']);
  f.followed(R, 0, msg(140, [{ id: 1, hour: 96, value: 'v96' }]));
  assert.deepEqual(f.window(1, 124), [96, Infinity], 'selected at 124 and valid from 96: the page at 100 shows it through 100');
  f.followed(R, 0, msg(150, [{ id: 1, hour: 141, value: 'v141' }]));
  assert.deepEqual(f.window(1, 124), [96, 140]);
  assert.deepEqual(f.window(1, 145), [141, Infinity]);
  const g = facts([1, 'panel']);
  g.panels(R, 0, [data(1, 10, { cells: [{ id: A }] }), stands(1, 30)]);
  g.followed(R, 0, msg(18, [{ id: A, hour: 10, value: 'a' }]));
  g.followed(R, 0, msg(25, [{ id: A, hour: 25, value: 'b' }]));
  assert.deepEqual([hc(g.select(1, 20)), g.window(1, 20)], [[10, 18], [10, 24]], 'the fallback holds until the next complete join starts');
  assert.deepEqual([hc(g.select(1, 26)), g.window(1, 26)], [[25, 25], [25, Infinity]]);
});

test('one completed step covers every joined product; an unchanged answer is covered by later messages; a seek clears', () => {
  const f = facts([1, 'panel'], [2, 'panel'], [3, 'panel'], [4, 'followed']);
  f.panels(R, 0, [data(1, 10, { stepped: true }), data(2, 10, { stepped: true }), data(3, 10)]);
  f.panels(R, 0, [], 34);
  assert.deepEqual([1, 2, 3].map(id => Math.min(33, f.select(id, 33)!.c)), [33, 33, 10], 'displayed through at 33 is min(33, c)');
  assert.deepEqual([1, 2, 3].map(id => f.select(id, 34)!.c), [34, 34, 10]);
  f.followed(R, 0, msg(34, [{ id: 4, hour: 34, value: 7 }]));
  f.followed(R, 0, msg(58));
  assert.equal(f.select(4, 58)!.c, 58);
  f.seek(R, 1, 0);
  assert.ok([1, 2, 3, 4].every(id => f.select(id, 0) === null));
});

test('a pending listed log limits only its own reading', () => {
  const f = facts([1, 'followed'], [2, 'followed']);
  f.followed(R, 0, msg(10, [{ id: 1, hour: 10, value: 7 }, { id: 2, hour: 10, value: 7 }]));
  f.followed(R, 0, msg(40, [], { limited: new Map([[1, 10]]) }));
  assert.deepEqual([1, 2].map(id => f.select(id, 40)!.c), [10, 40]);
});

test('a stepped panel covers each completed step until its next reading', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { stepped: true, value: 'held' })]);
  f.panels(R, 0, [], 22);
  f.panels(R, 0, [data(1, 23, { value: 'changed' })]);
  assert.deepEqual([hc(f.select(1, 22)), f.select(1, 22)!.value], [[10, 22], 'held']);
  assert.deepEqual(hc(f.select(1, 23)), [23, 23]);
  const g = facts([1, 'panel']);
  g.panels(R, 0, [data(1, 110, { stepped: true, value: 'held' })], 122);
  assert.deepEqual(hc(g.select(1, 130)), [110, 122], 'coverage stops at the completed step');
  assert.equal(Math.min(112, g.select(1, 112)!.c), 112, 'and is displayed through the target, never past it');
});

test('a product waiting for its table is covered only through the hour before it waited', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { stepped: true, value: 'held' })]);
  f.panels(R, 0, [], 22);
  f.panels(R, 0, [waiting(1, 34)]);
  f.panels(R, 0, [], 34); f.panels(R, 0, [], 58);
  f.panels(R, 0, [data(1, 82, { value: 'changed' })]);
  assert.deepEqual(hc(f.select(1, 58)), [10, 33]);
  assert.deepEqual(hc(f.select(1, 82)), [82, 82]);
});

test('a native reset keeps an old panel to the hours it was confirmed through until its replacement shows', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { stepped: true, value: 'held' })]);
  f.panels(R, 0, [waiting(1, 34)], 34);
  assert.deepEqual(hc(f.select(1, 58)), [10, 33]);
  f.followed(R, 0, msg(58, [], { reset: true }));
  f.panels(R, 0, [waiting(1, 58)], 58);
  f.panels(R, 0, [data(1, 58, { stepped: true, cells: [{ id: A, seedHour: 99, values: [12] }] })], 58);
  const p = f.select(1, 58);
  assert.ok(p === null || p.c <= 33, JSON.stringify(hc(p)));
});

test('a reading answered again after a native reset advances with its lane', () => {
  const f = facts([1, 'followed']);
  f.followed(R, 0, msg(10, [{ id: 1, hour: 10, value: 7 }]));
  f.followed(R, 0, msg(20, [{ id: 1, hour: 20, value: 7 }], { reset: true }));
  f.followed(R, 0, msg(34));
  assert.deepEqual(hc(f.select(1, 34)), [20, 34]);
});

test('seeks admit their own records; a native reset supersedes its queued future; a forward seek beyond coverage waits', () => {
  const f = facts([1, 'followed'], [2, 'panel']);
  f.seek(R, 17, 10);
  f.followed(R, 17, msg(30, [{ id: 1, hour: 30, value: 30 }]));
  f.followed(R, 17, msg(10, [{ id: 1, hour: 10, value: 10 }], { reset: true }));
  f.panels(R, 17, [data(2, 10, { value: 'current' })]);
  assert.deepEqual([f.select(1, 30)!.h, f.select(2, 30)!.value], [10, 'current']);
  f.seek(R, 18, 6);
  f.followed(R, 17, msg(6, [{ id: 1, hour: 6, value: 6 }], { reset: true }));
  f.panels(R, 17, [data(2, 5, { value: 'old seek' })]);
  assert.deepEqual([f.select(1, 6), f.select(2, 6)], [null, null]);
  f.followed(R, 18, msg(6, [{ id: 1, hour: 6, value: 6 }], { reset: true }));
  f.panels(R, 18, [data(2, 6, { value: 'new seek' })]);
  assert.deepEqual([f.select(1, 6)!.h, f.select(2, 6)!.value], [6, 'new seek']);
  f.seek(R, 19, 50);
  assert.equal(f.select(2, 50), null, 'nothing of the new stream yet: the lane presents "Reading hour 50"');
  f.panels(R, 19, [data(2, 50, { value: 'replacement' })]);
  assert.equal(f.select(2, 50)!.value, 'replacement');
});

test('a scrub keeps the confirmed interval containing the target', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { stepped: true, value: 'still current' })], 22);
  f.seek(R, 1, 17);
  const p = f.select(1, 17)!;
  assert.deepEqual([hc(p), p.value], [[10, 22], 'still current']);
});

test('an early panel leaves the eligible picture up until its hour; a seek drops the abandoned generation', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { value: 'early' }), data(1, 13, { value: 'later' })]);
  assert.equal(f.select(1, 10)!.value, 'early');
  assert.equal(f.select(1, 13)!.value, 'later');
  f.seek(R, 1, 5);
  f.panels(R, 0, [data(1, 5, { value: 'abandoned generation' })]);
  assert.equal(f.select(1, 5), null);
  f.panels(R, 1, [data(1, 5, { value: 'after seek' })]);
  assert.equal(f.select(1, 5)!.value, 'after seek');
});

test('exact slot snapshots stay paintable when their live stream has passed the read', () => {
  const f = facts([1, 'panel']);
  const painted: number[] = [];
  for (const hour of [10, 20, 30]) {
    f.followed(R, 0, msg(hour + 2, [{ id: A, hour: hour + 2, value: 90 }]));
    f.panels(R, 0, [data(1, hour, { cells: [{ id: A, seedHour: hour, values: hour }] })]);
    const p = f.select(1, 100)!;
    painted.push(p.c);
    assert.equal(p.parts[1]!.value, hour, 'the exact progress belongs to the painted hour');
  }
  assert.deepEqual(painted, [10, 20, 30]);
});

test('a panel and two independently confirmed cells meet at their shared coverage', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 12, { cells: [{ id: A, seedHour: 10, values: [12] }, { id: B, seedHour: 12, values: [12] }] }), stands(1, 30)]);
  f.followed(R, 0, msg(22, [{ id: A, hour: 10, value: 12 }]));
  assert.deepEqual(hc(f.select(1, 40)), [12, 12], 'the second seed confirms only its own hour');
  f.followed(R, 0, msg(24, [{ id: B, hour: 12, value: 12 }]));
  assert.deepEqual(hc(f.select(1, 40)), [12, 24]);
  f.panels(R, 0, [data(1, 25, { cells: [{ id: A, seedHour: 25, values: [12] }, { id: B, seedHour: 25, values: [12] }] }), stands(1, 40)]);
  f.followed(R, 0, msg(29));
  assert.deepEqual(hc(f.select(1, 40)), [25, 29], 'an established stream confirms a newer exact seed');
  f.followed(R, 0, msg(30, [{ id: B, hour: 34, value: 12 }]));
  assert.deepEqual(hc(f.select(1, 30)), [25, 29], 'a cell reading ahead of the common head waits');
  assert.deepEqual(hc(f.select(1, 34)), [25, 29], 'the two readings still share no hour');
  f.followed(R, 0, msg(35));
  assert.deepEqual(hc(f.select(1, 35)), [34, 35]);
});

function pausedCells(offset: number, change: number, later?: number) {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, offset + 10, { stepped: true, cells: [{ id: A, seedHour: offset + 10, values: [5] }] })]);
  f.followed(R, 0, msg(offset + 12, [{ id: A, hour: offset + 10, value: 5 }]));
  for (let at = offset + 13; at <= offset + 40; at++) {
    const answers = at === offset + change ? [{ id: A, hour: at, value: 7 }] : at === offset + (later ?? -1) ? [{ id: A, hour: at, value: 9 }] : [];
    f.followed(R, 0, msg(at, answers)); f.panels(R, 0, [], at);
  }
  return f;
}
const covers = (p: Picture<V> | null, target: number) => !!p && p.h <= target && target <= p.c;

test('a pause keeps a stepped panel whose cells cover the paused hour; a reset and a resend at that hour keep it', () => {
  const f = pausedCells(0, 15);
  const p = f.select(1, 20)!;
  assert.ok(covers(p, 20)); assert.equal(p.parts[1]!.value, 7);
  f.followed(R, 0, msg(20, [{ id: A, hour: 20, value: 7 }], { reset: true }));
  assert.ok(covers(f.select(1, 20), 20), "native's reseek at the paused hour keeps it");
  f.panels(R, 0, [data(1, 20, { stepped: true, cells: [{ id: A, seedHour: 20, values: [7] }] })]);
  assert.ok(covers(f.select(1, 20), 20), 'the panel sent again keeps it');
});

test('and :588 a pause just ahead of the drawn hour keeps what arrived for it, on the axis or an offset track', () => {
  for (const offset of [0, 100]) {
    const p = pausedCells(offset, 19, 30).select(1, offset + 20);
    assert.ok(covers(p, offset + 20)); assert.equal(p!.parts[1]!.value, 7, 'the value for hour 19, never the one for 30');
  }
});

test('a pause keeps a stepped panel when native sends the panel again before its cells', () => {
  const f = pausedCells(0, 15);
  f.panels(R, 0, [data(1, 20, { stepped: true, cells: [{ id: A, seedHour: 20, values: [7] }] })]);
  assert.ok(covers(f.select(1, 20), 20));
  f.followed(R, 0, msg(20, [{ id: A, hour: 20, value: 7 }], { reset: true }));
  assert.ok(covers(f.select(1, 20), 20));
});

test('a stepped panel whose cells are followed stands through each completed step', () => {
  const f = facts([3, 'panel']);
  f.panels(R, 0, [data(3, 10, { stepped: true, cells: [{ id: A, seedHour: 10, values: [7] }] })], 10);
  assert.equal(f.select(3, 10)!.c, 10);
  f.followed(R, 0, msg(11, [{ id: A, hour: 11, value: 8 }]));
  f.panels(R, 0, [], 11);
  const p = f.select(3, 11)!;
  assert.deepEqual([p.c, p.parts[1]!.value], [11, 8]);
});

test('a shown panel stands only until a new panel waiting on its first cell starts', () => {
  const f = facts([3, 'panel']);
  f.panels(R, 0, [data(3, 10, { stepped: true, cells: [{ id: A, seedHour: 10, values: [7] }] })], 10);
  f.followed(R, 0, msg(11, [{ id: A, hour: 11, value: 8 }])); f.panels(R, 0, [], 11);
  f.panels(R, 0, [data(3, 12, { stepped: true, cells: [{ id: B }] })], 12);
  f.followed(R, 0, msg(14, [{ id: A, hour: 14, value: 9 }])); f.panels(R, 0, [], 14);
  assert.equal(f.select(3, 14)!.c, 11);
  f.panels(R, 0, [data(3, 20, { stepped: true, cells: [{ id: C }] })], 20);
  f.followed(R, 0, msg(20, [{ id: A, hour: 20, value: 10 }]));
  assert.equal(f.select(3, 20)!.c, 11, 'a later waiting panel never moves the end of the shown one');
});

test('review 3: a same-seek reset withdraws a carried follow picture but keeps a carried panel', () => {
  const f = facts([1, 'followed'], [2, 'panel']);
  f.followed(R, 0, msg(40, [{ id: 1, hour: 10, value: 'follow' }]));
  f.panels(R, 0, [data(2, 10, { stepped: true })], 40);
  f.seek(R, 1, 20);
  assert.deepEqual([f.select(1, 20)?.carried, f.select(2, 20)?.carried], [true, true]);
  f.followed(R, 1, msg(20, [], { reset: true }));
  assert.equal(f.select(1, 20), null, 'the withdrawn follow reading is gone with the reset');
  assert.equal(f.select(2, 20)!.value, '2@10', 'the panel fact is independently valid and stays');
});

test('review 3: one unchanged panel with one changing cell keeps only the cell versions a picture can still use', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 0, { stepped: true, cells: [{ id: A }] })]);
  let installed: Picture<V> | null = null;
  for (let hour = 0; hour < 1000; hour++) {
    f.followed(R, 0, msg(hour, [{ id: A, hour, value: hour, bytes: 100 }]));
    f.panels(R, 0, [], hour);
    installed = f.select(1, hour);
    f.prune(1, hour, [installed]);
  }
  assert.equal(installed!.parts[1]!.value, 999);
  assert.ok(f.retained(1).cells <= 2, `retained ${f.retained(1).cells} cell facts`);
  assert.ok(f.releasedBytes() >= 99_800, 'released versions return their bytes as credit');
});

test('review 3: pruning at the actual target never changes a selection at or after it, under late and early statements', () => {
  let seed = 0x2545f491;
  const rnd = (n: number) => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return (seed >>> 0) % n; };
  for (let run = 0; run < 300; run++) {
    const pruned = facts([1, 'panel']), whole = facts([1, 'panel']);
    const both = (act: (f: Facts<V>) => void) => { act(pruned); act(whole); };
    let hour = rnd(5), actual = 0, next = A;
    let live: number[] = [];
    const installed: (Picture<V> | null)[] = [];
    for (let op = 0; op < 50; op++) {
      hour += rnd(4);
      const k = rnd(10);
      if (k < 2) {
        const named = [...live.filter(() => rnd(3) > 0), ...Array.from({ length: rnd(2) }, () => next++)].slice(0, 2);
        live = named;
        const cells = named.map(id => rnd(2) ? { id, seedHour: hour - rnd(3), values: hour } : { id });
        const stepped = !!rnd(2);
        both(f => f.panels(R, 0, [data(1, hour, { stepped, cells })]));
      }
      else if (k < 3) { const stepped = !!rnd(2); both(f => f.panels(R, 0, [stands(1, hour, stepped)])); }
      else if (k < 4) both(f => f.panels(R, 0, [waiting(1, hour)]));
      else if (k < 5) both(f => f.panels(R, 0, [], hour));
      else { const answers = live.filter(() => rnd(3) === 0).map(id => ({ id, hour: hour - rnd(4), value: hour })); both(f => f.followed(R, 0, msg(hour, answers))); }
      if (rnd(3) === 0) {
        actual = Math.max(actual, hour - rnd(6));
        installed.splice(0, installed.length, pruned.select(1, actual));
        pruned.prune(1, actual, installed);
      }
      for (let target = actual; target <= hour + 3; target++)
        assert.equal(pruned.select(1, target)?.c ?? null, whole.select(1, target)?.c ?? null, `run ${run} op ${op} target ${target}`);
    }
  }
});

test('credit: facts dropped by a seek, a reset or an older stream return their bytes', () => {
  const f = facts([1, 'followed']);
  f.followed(R, 0, msg(10, [{ id: 1, hour: 10, value: 1, bytes: 40 }]));
  f.followed(R, 0, msg(20, [{ id: 1, hour: 20, value: 2, bytes: 60 }], { reset: true }));
  assert.equal(f.releasedBytes(), 40);
  f.seek(R, 1, 30);
  assert.equal(f.releasedBytes(), 60);
  f.followed(R, 0, msg(40, [{ id: 1, hour: 40, value: 3, bytes: 7 }]));
  assert.equal(f.releasedBytes(), 7, 'an older stream never lands, and its bytes come back at once');
});

test('pinned reads are exact facts outside every route stream', () => {
  const f = new Facts<V>();
  f.follow(5, -1, 'pinned');
  f.exact(5, 50, 'pinned value');
  assert.deepEqual(hc(f.select(5, 50)), [50, 50]);
  assert.equal(f.select(5, 49), null);
});

test('addendum 1: a window also breaks at the product change hours, and the painter hour is the window start', () => {
  const nextDay = (hour: number) => (Math.floor(hour / 24) + 1) * 24;
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 96, { stepped: true })], 140);
  const window = f.window(1, 124)!;
  assert.deepEqual(window, [96, Infinity], 'past its confirmation the fallback is still this picture, aged by the page');
  assert.deepEqual(changeWindow(window, 124, nextDay), [120, 143], 'at 124 the stepped panel of 96 paints its hour-120 day count');
  assert.deepEqual(changeWindow(window, 100, nextDay), [96, 119]);
  assert.deepEqual(changeWindow(window, 130, nextDay, 120), [120, 143], 'stepping on from a cached start');
  assert.deepEqual(changeWindow([96, 130], 124, nextDay), [120, 130]);
  const reveals = [100, 110, 125];
  const nextReveal = (hour: number) => reveals.find(at => at > hour) ?? Infinity;
  assert.deepEqual(changeWindow(window, 124, nextReveal), [110, 124], 'rows revealed by 124 and none later');
  assert.throws(() => changeWindow(window, 124, () => 50), /does not follow/);
});

test('addendum 3: a widget over several subscriptions joins with the latest hour, the earliest confirmation and the common window', () => {
  const f = facts([1, 'followed'], [2, 'followed']);
  f.followed(R, 0, msg(30, [{ id: 1, hour: 10, value: 'a' }, { id: 2, hour: 20, value: 'b' }]));
  f.followed(R, 0, msg(40, [{ id: 1, hour: 35, value: 'a2' }], { limited: new Map([[2, 30]]) }));
  const at = (target: number) => joinComponents([1, 2].map(id => ({ picture: f.select(id, target)!, window: f.window(id, target)! })));
  assert.deepEqual(at(25), { h: 20, c: 30, window: [20, 34] }, 'shown through min(t, 30) until the first subscription changes at 35');
  assert.equal(at(40), null, 'A of 35 and B confirmed only through 30 share no certified hour: nothing new installs, the previous complete picture stays');
  assert.equal(joinComponents([
    { picture: f.select(1, 25)!, window: [10, 15] }, { picture: f.select(2, 25)!, window: [20, 30] }]), null, 'an empty intersection is not installable');
});

interface Statement { kind: 'data' | 'stands' | 'waiting' | 'completed' | 'follow'; hour: number; stepped?: boolean; cells?: { id: number; seed?: number }[];
  answers?: { id: number; hour: number }[]; limited?: [number, number][] }

function oracleSelect(statements: readonly Statement[], target: number): number | null {
  interface Iv { h: number; c: number }
  const panel: (Iv & { cells: { id: number; seed?: number }[] })[] = [];
  const cells = new Map<number, Iv[]>();
  let open = false, stepped = false;
  const cellOpen = new Map<number, boolean>();
  for (const s of statements) {
    if (s.kind === 'data') {
      const last = panel[panel.length - 1];
      if (last && last.h === s.hour) panel.pop();
      else if (last && last.c >= s.hour) last.c = s.hour - 1;
      panel.push({ h: s.hour, c: s.hour, cells: s.cells ?? [] });
      open = true; if (s.stepped) stepped = true;
      for (const cell of s.cells ?? []) if (!cells.has(cell.id)) cells.set(cell.id, []);
    } else if (s.kind === 'stands') {
      const last = panel[panel.length - 1];
      if (s.stepped) stepped = true;
      if (last) { last.c = Math.max(last.c, s.hour); open = true; }
    } else if (s.kind === 'waiting') {
      const last = panel[panel.length - 1];
      if (last && open) last.c = Math.max(last.c, s.hour - 1);
      open = false;
    } else if (s.kind === 'completed') {
      const last = panel[panel.length - 1];
      if (last && open && stepped && last.c < s.hour) last.c = s.hour;
    } else {
      const answered = new Set<number>();
      const limit = new Map(s.limited ?? []);
      for (const answer of s.answers ?? []) {
        const list = cells.get(answer.id);
        if (!list) continue;
        answered.add(answer.id);
        const last = list[list.length - 1];
        const fact = { h: answer.hour, c: Math.max(answer.hour, limit.get(answer.id) ?? s.hour) };
        if (last && last.h === fact.h) list[list.length - 1] = fact;
        else { if (last && last.c >= fact.h) last.c = fact.h - 1; list.push(fact); }
        cellOpen.set(answer.id, true);
      }
      for (const [id, list] of cells) if (!answered.has(id) && cellOpen.get(id)) {
        const last = list[list.length - 1], confirmed = limit.get(id) ?? s.hour;
        if (last && confirmed > last.c) last.c = confirmed;
      }
    }
  }
  const joins = (fact: (typeof panel)[number]) => {
    let out: Iv[] = [{ h: fact.h, c: fact.c }];
    for (const cell of fact.cells) {
      const list = [...cells.get(cell.id) ?? [], ...cell.seed !== undefined && cell.seed <= fact.h ? [{ h: cell.seed, c: cell.seed }] : []];
      out = out.flatMap(j => list.map(x => ({ h: Math.max(j.h, x.h), c: Math.min(j.c, x.c) }))).filter(j => j.h <= j.c);
    }
    return out;
  };
  const due = panel.filter(x => x.h <= target);
  const latest = due[due.length - 1];
  if (latest) { const covering = joins(latest).filter(j => j.h <= target && target <= j.c); if (covering.length) return Math.max(...covering.map(j => j.c)); }
  const all = due.flatMap(joins).filter(j => j.h <= target);
  return all.length ? Math.max(...all.map(j => j.c)) : null;
}

test('oracle: selection matches a brute-force enumeration over an independent replay of random statements', () => {
  let seed = 0x9e3779b9;
  const rnd = (n: number) => { seed ^= seed << 13; seed ^= seed >>> 17; seed ^= seed << 5; return (seed >>> 0) % n; };
  let checks = 0;
  const seen = { none: 0, covering: 0, fallback: 0, joined: 0 };
  for (let run = 0; run < 400; run++) {
    const f = facts([1, 'panel']);
    const statements: Statement[] = [];
    let hour = rnd(5);
    for (let op = 0; op < 40; op++) {
      hour += rnd(4);
      const k = rnd(10);
      if (k < 2) {
        const cells = [A, B, C].filter(() => rnd(2)).map(id => rnd(2) ? { id, seed: hour - rnd(3) } : { id });
        const stepped = !!rnd(2);
        statements.push({ kind: 'data', hour, stepped, cells });
        f.panels(R, 0, [data(1, hour, { stepped, cells: cells.map(cell => cell.seed === undefined ? { id: cell.id } : { id: cell.id, seedHour: cell.seed, values: cell.seed }) })]);
      } else if (k < 3) { const stepped = !!rnd(2); statements.push({ kind: 'stands', hour, stepped }); f.panels(R, 0, [stands(1, hour, stepped)]); }
      else if (k < 4) { statements.push({ kind: 'waiting', hour }); f.panels(R, 0, [waiting(1, hour)]); }
      else if (k < 5) { statements.push({ kind: 'completed', hour }); f.panels(R, 0, [], hour); }
      else {
        const answers = [A, B, C].filter(() => rnd(3) === 0).map(id => ({ id, hour: hour - rnd(4) }));
        const limited: [number, number][] | undefined = rnd(4) ? undefined : [[[A, B, C][rnd(3)]!, hour - rnd(6)]];
        statements.push({ kind: 'follow', hour, answers, limited });
        f.followed(R, 0, msg(hour, answers.map(a => ({ ...a, value: hour })), { limited: limited && new Map(limited) }));
      }
      for (let target = hour - 8; target <= hour + 2; target++) {
        const got = f.select(1, target), want = oracleSelect(statements, target);
        assert.equal(got?.c ?? null, want, `run ${run} op ${op} target ${target}`);
        seen[!got ? 'none' : got.h <= target && target <= got.c ? 'covering' : 'fallback']++;
        if (got && got.parts.length > 2) seen.joined++;
        if (got) {
          assert.ok(got.h <= target, 'never ahead');
          assert.ok(got.parts.every(x => x.h <= got.h && got.c <= x.c), 'every part covers the picture');
          assert.ok(samePicture(got, f.select(1, target)), 'selection is pure');
        }
        checks++;
      }
    }
  }
  assert.ok(checks > 100000 && Object.values(seen).every(n => n > 5000), JSON.stringify(seen));
});

test('a seek keeps panel cell groups owned until the post-seek panel lands, even across an intervening prune', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { stepped: true, cells: [{ id: A }] })], 10);
  f.followed(R, 0, msg(10, [{ id: A, hour: 10, value: 'v0' }]));
  assert.equal(f.owns(A), true);
  f.seek(R, 1, 50);
  assert.deepEqual([f.owns(A), f.drain()], [true, []], 'cell group stays owned while waiting for seek 1 panel');
  f.prune(1, 50, []);
  assert.deepEqual([f.owns(A), f.drain()], [true, []], 'pruning before seek 1 panel lands does not release cell group');
  f.followed(R, 1, msg(50, [{ id: A, hour: 50, value: 'v1' }], { reset: true }));
  assert.equal(f.latest(A), 'v1', 'reset whole table lands directly before seek 1 panel arrives');
  f.panels(R, 1, [data(1, 50, { stepped: true, cells: [{ id: A }] })], 50);
  assert.equal(f.select(1, 50)?.parts[1]?.value, 'v1');
  f.panels(R, 1, [data(1, 60, { stepped: true, cells: [] })], 60);
  f.prune(1, 60, [f.select(1, 60)]);
  f.panels(R, 1, [data(1, 70, { stepped: true, cells: [] })], 70);
  f.prune(1, 70, [f.select(1, 70)]);
  assert.deepEqual(f.drain(), [A], 'once the new panel no longer uses the cell group, prune releases it');
});

test('S10: nextStart includes future cell-group starts when a panel covers target without a matching cell yet', () => {
  const f = facts([1, 'panel']);
  f.panels(R, 0, [data(1, 10, { stepped: true, cells: [{ id: A }] })], 30);
  f.followed(R, 0, msg(20, [{ id: A, hour: 20, value: 'v20' }]));
  assert.equal(f.select(1, 15), null);
  assert.equal(f.nextStart(1, 15), 20, 'nextStart sees the unseeded cell group becoming available at hour 20');
});
