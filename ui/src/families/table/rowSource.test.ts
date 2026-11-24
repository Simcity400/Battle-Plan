import assert from 'node:assert/strict';
import test from 'node:test';
import { buildNavigableSlots, TailWindowController, validateTailReply, type DatedLogRow, type TailReply } from './rowSource.ts';

interface Row extends DatedLogRow {
  readonly label: string;
}

const makeReply = (rows: readonly Row[], total: number, missing: TailReply<Row>['missing'] = []): TailReply<Row> => ({
  total,
  offset: 0,
  baseOrdinal: 0,
  firstOrdinal: 0,
  rows,
  facetTotals: new Map(),
  missing,
});

test('validateTailReply rejects malformed replies and rows after the supplied displayed hour', () => {
  const raw = {
    columns: ['kind'],
    total: '2',
    offset: '0',
    baseOrdinal: '0',
    firstOrdinal: '0',
    rows: [
      { key: '2', hour: 120, cells: [{ kind: 'number', value: '11' }] },
      { key: '1', hour: 100, cells: [{ kind: 'number', value: '20' }] },
    ],
    facetTotals: [{ key: 'contracts', total: '1' }],
    missing: [{ firstHour: 105, lastHour: 110, reason: 'gap' }],
  };
  const ok = validateTailReply(raw, 120);
  assert.equal(ok.total, 2);
  assert.equal(ok.facetTotals.get('contracts'), 1);
  assert.equal(ok.rows.length, 2);
  assert.throws(() => validateTailReply(raw, 119), /after displayed hour 119/);
  assert.throws(() => validateTailReply({ ...raw, total: '1' }, 120), /malformed/);
  assert.throws(() => validateTailReply({ ...raw, total: '-1' }, 120), /malformed/);
});

test('sliceAt tags every slot with its readAt hour and never returns rows for a different or future hour', () => {
  const controller = new TailWindowController<Row>('q1', 2);
  controller.applyHead(120, makeReply([
    { key: 'a', hour: 120, label: 'newest' },
    { key: 'b', hour: 110, label: 'older' },
  ], 4));

  const snap = controller.snapshot();
  const atDisplayed = snap.sliceAt(120, 0, 3);
  assert.deepEqual(atDisplayed.map(slot => [slot.kind, slot.readAt]), [
    ['data', 120],
    ['data', 120],
    ['loading', 120],
  ]);

  const atOtherHour = snap.sliceAt(119, 0, 2);
  assert.deepEqual(atOtherHour.map(slot => [slot.kind, slot.readAt]), [
    ['loading', 119],
    ['loading', 119],
  ]);

  assert.throws(() => controller.applyHead(115, makeReply([{ key: 'future', hour: 116, label: 'bad' }], 1)), /exceeds acknowledged hour 115/);
});

test('scrolling freezes at the acknowledged hour, refuses pages distinctly from gaps, and releases on backward seek, top return, or ADR 1401 forceFull', () => {
  const controller = new TailWindowController<Row>('q1', 2);
  controller.applyHead(200, makeReply([
    { key: 'r0', hour: 200, label: '0' },
    { key: 'r1', hour: 190, label: '1' },
  ], 6));

  const plan = controller.setWindow({ first: 2, count: 4 });
  assert.equal(controller.frozen?.hour, 200);
  assert.deepEqual(plan.fetch, [
    { offset: 2, limit: 2, hour: 200 },
    { offset: 4, limit: 2, hour: 200 },
  ]);

  assert.equal(controller.applyPage(200, 2, [
    { key: 'r2', hour: 180, label: '2' },
    { key: 'r3', hour: 170, label: '3' },
  ]), true);
  assert.equal(controller.refusePage(200, 4, 'Market events block damaged'), true);

  controller.applyHead(250, makeReply([
    { key: 'r_new', hour: 250, label: 'new' },
    { key: 'r0', hour: 200, label: '0' },
  ], 7));
  assert.equal(controller.frozen?.hour, 200, 'forward playback does not disturb the frozen window');

  const slots = controller.snapshot().sliceAt(200, 2, 4);
  assert.deepEqual(slots.map(slot => slot.kind), ['data', 'data', 'refused', 'refused']);
  assert.equal(slots[2]!.kind === 'refused' ? slots[2]!.reason : null, 'Market events block damaged');
  assert.equal(slots[2]!.readAt, 200);

  const cancelled = controller.applyHead(260, makeReply([
    { key: 'r_ff', hour: 260, label: 'ff' },
    { key: 'r_new', hour: 250, label: 'new' },
  ], 8), 0, { forceFull: true });
  assert.equal(controller.frozen, null, 'ADR 1401 forceFull releases the freeze');
  assert.deepEqual(cancelled, []);

  controller.setWindow({ first: 2, count: 2 });
  assert.equal(controller.snapshot().frozen?.hour, 260);
  assert.equal(controller.applyPage(200, 2, [{ key: 'stale', hour: 180, label: 'stale' }]), false);

  const backCancelled = controller.applyHead(150, makeReply([{ key: 'r_back', hour: 150, label: 'back' }], 1));
  assert.equal(controller.frozen, null, 'backward seek past frozen hour releases freeze');
  assert.deepEqual(backCancelled, [2]);

  controller.refuseHead(160, 'Log table withheld for this recording');
  const refusedSnap = controller.snapshot();
  assert.equal(refusedSnap.refused, 'Log table withheld for this recording');
  assert.deepEqual(refusedSnap.sliceAt(160, 0, 3), [
    { kind: 'refused', ordinal: 0, readAt: 160, reason: 'Log table withheld for this recording' },
  ]);
});

test('buildNavigableSlots interleaves recording gaps in newest-first order and keeps them when total is zero', () => {
  const gaps = [
    { firstHour: 130, lastHour: 140, reason: 'gap late' },
    { firstHour: 100, lastHour: 110, reason: 'gap early' },
  ];
  const empty = buildNavigableSlots<Row>(135, [], gaps, 0);
  assert.deepEqual(empty.map(slot => slot.kind === 'gap' ? [slot.firstHour, slot.lastHour, slot.readAt] : null), [
    [130, 135, 135],
    [100, 110, 135],
  ]);

  const data = [
    { kind: 'data' as const, ordinal: 0, readAt: 150, row: { key: 'a', hour: 145, label: 'a' } },
    { kind: 'data' as const, ordinal: 1, readAt: 150, row: { key: 'b', hour: 120, label: 'b' } },
  ];
  const mixed = buildNavigableSlots(150, data, gaps, 2);
  assert.deepEqual(mixed.map(slot => slot.kind === 'data' ? slot.row.key : slot.kind === 'gap' ? `${slot.firstHour}-${slot.lastHour}@${slot.beforeOrdinal}` : slot.kind), [
    'a',
    '130-140@1',
    'b',
    '100-110@2',
  ]);

  const scrolledWindow = [
    { kind: 'data' as const, ordinal: 2, readAt: 150, row: { key: 'c', hour: 120, label: 'c' } },
  ];
  const scrolled = buildNavigableSlots(150, scrolledWindow, gaps, 3);
  assert.deepEqual(scrolled.map(slot => slot.kind === 'data' ? slot.row.key : slot.kind === 'gap' ? `${slot.firstHour}-${slot.lastHour}@${slot.beforeOrdinal}` : slot.kind), [
    'c',
    '100-110@3',
  ]);
});
