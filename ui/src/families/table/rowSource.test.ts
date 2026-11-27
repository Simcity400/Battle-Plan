import assert from 'node:assert/strict';
import test from 'node:test';
import { PayloadSender } from '../../dashboard/follow/payloads.ts';
import { tableHostAdapter } from './hostAdapter.ts';
import { gatedHour } from '../../test-support/gated.ts';
import { projectPagedSlots, tailFollowParams } from './logHelpers.ts';
import { formatDateOnly } from '../valueFormat.ts';
import { buildNavigableSlots, buildRowSourceFromHead, SnapshotPager, validateGenericTailReply, validateRowTailReply, type DatedLogRow, type SnapshotPage, type SnapshotPages, type TailReply } from './rowSource.ts';

interface Row extends DatedLogRow {
  readonly label: string;
}

const makeReply = (rows: readonly Row[], total: number, missing: TailReply<Row>['missing'] = []): TailReply<Row> => ({
  total,
  offset: 0,
  rows,
  facetTotals: new Map(),
  missing,
});

const rawReply = (rows: readonly DatedLogRow[], total: number, extra: Record<string, unknown> = {}) => ({
  total: String(total), offset: '0', rows, missing: [], counts: [], ...extra,
});
const page = (hour: number, offset: number, limit: number, rows: readonly DatedLogRow[], total: number, counts: { key: string; total: string }[] = []): SnapshotPage => {
  const raw = rawReply(rows, total, { offset: String(offset), counts });
  return { hour, offset, limit, raw, reply: validateGenericTailReply(raw, hour) };
};

test('the log refuses a missing or empty row identity and keeps the next recorded row', () => {
  const raw = { total: '3', offset: '0', rows: [{ hour: 100 }, { key: '', hour: 100 }, { key: 'third', hour: 98 }] };
  const source = buildRowSourceFromHead('log', raw, undefined, 100)!;
  const slots = source.sliceAt(100, 0, 3);
  assert.deepEqual(slots, [
    { kind: 'refused', ordinal: 0, readAt: 100, hour: 100, reason: 'The log row identity was not recorded.' },
    { kind: 'refused', ordinal: 1, readAt: 100, hour: 100, reason: 'The log row identity was not recorded.' },
    { kind: 'data', ordinal: 2, readAt: 100, row: { key: 'third', hour: 98 } },
  ]);
  const withGap = buildNavigableSlots(100, slots, [{ firstHour: 99, lastHour: 99, reason: 'Recording gap' }], 3);
  assert.deepEqual(withGap.map(slot => slot.kind), ['refused', 'refused', 'gap', 'data']);
});

test('a scrolled log places a recording gap using the recorded hour of a refused row', () => {
  const raw = { total: '3', offset: '1', rows: [{ key: '', hour: 100 }, { key: 'next', hour: 98 }] };
  const source = buildRowSourceFromHead('log', raw, undefined, 100)!;
  const slots = source.sliceAt(100, 1, 2);
  const withGap = buildNavigableSlots(100, slots, [{ firstHour: 99, lastHour: 99, reason: 'Recording gap' }], 3);
  assert.deepEqual(withGap.map(slot => slot.kind), ['refused', 'gap', 'data']);
});

test('a scrolled log is one snapshot: total, chips and rows come from the pages of one hour, never mixed with a later live head', () => {
  const head = rawReply([{ key: 'new', hour: 101 }, { key: 'r0', hour: 100 }, { key: 'r1', hour: 99 }], 7);
  const pages: SnapshotPages = {
    0: page(100, 0, 4, [{ key: 'r0', hour: 100 }, { key: 'r1', hour: 99 }, { key: 'r2', hour: 98 }, { key: 'r3', hour: 97 }], 6, [{ key: 'running', total: '2' }]),
    4: { hour: 100, offset: 4, limit: 4, refused: 'Market events block damaged' },
  };
  const scrolled = buildRowSourceFromHead('q', head, undefined, 101, pages)!;
  assert.equal(scrolled.at, 100, 'the source carries its snapshot hour');
  assert.equal(scrolled.total, 6, 'the total is the snapshot’s, not the live head’s');
  assert.equal(scrolled.facetTotals?.get('running'), 2);
  assert.deepEqual(scrolled.sliceAt(101, 0, 7).map(slot => (slot.kind === 'data' ? slot.row.key : slot.kind)), ['r0', 'r1', 'r2', 'r3', 'refused', 'refused'], 'every ordinal is the snapshot’s own row, and a refused page is refused');

  const seekBack = rawReply([{ key: 'r5', hour: 90 }], 3);
  const before = buildRowSourceFromHead('q', seekBack, undefined, 90, pages)!;
  assert.equal(before.at, 90, 'a snapshot from after the displayed hour is not shown');
  assert.equal(before.total, 3);
  assert.deepEqual(before.sliceAt(90, 0, 3).map(slot => (slot.kind === 'data' ? slot.row.key : slot.kind)), ['r5', 'loading', 'loading'], 'the head at the displayed hour, the rest loading until the window is asked again');

  const refused = buildRowSourceFromHead('q', undefined, 'Log table withheld for this recording', 160)!;
  assert.equal(refused.refused, 'Log table withheld for this recording');
  assert.deepEqual(refused.sliceAt(160, 0, 3), [{ kind: 'refused', ordinal: 0, readAt: 160, reason: 'Log table withheld for this recording' }]);
});

test('the pager freezes one hour, asks it again after a seek before it, keeps pages only around the window, and an unchanged page keeps its object', () => {
  const pager = new SnapshotPager(4, 4);
  const answer = (ask: { offset: number; limit: number; hour: number }) => pager.answer(ask, makeReply([{ key: `p${ask.offset}`, hour: ask.hour, label: '' }], 100), {});
  assert.deepEqual(pager.window({ first: 0, count: 3 }, 100), { fetch: [], cancel: [] }, 'inside the head nothing is asked');
  assert.deepEqual(pager.window({ first: 1, count: 3 }, 100), { fetch: [], cancel: [] }, 'nor anywhere inside it');
  const first = pager.window({ first: 5, count: 6 }, 100);
  assert.deepEqual(first.fetch, [{ offset: 4, limit: 4, hour: 100 }, { offset: 8, limit: 4, hour: 100 }], 'pages on a grid from ordinal 0, at the frozen hour');
  assert.equal(answer(first.fetch[0]!), true);
  const held = pager.pages();
  assert.deepEqual(pager.window({ first: 5, count: 6 }, 150), { fetch: [], cancel: [] }, 'playing on keeps the frozen snapshot');
  assert.equal(pager.pages(), held, 'an unchanged window publishes the same pages object');
  const back = pager.window({ first: 5, count: 6 }, 90);
  assert.deepEqual([back.cancel, back.fetch.map(ask => [ask.offset, ask.hour])], [[8], [[4, 90], [8, 90]]], 'a seek before the frozen hour drops the snapshot and asks the window again at the displayed hour');
  assert.equal(answer(first.fetch[1]!), false, 'a page asked at the dropped hour is not installed');
  assert.deepEqual(Object.keys(pager.pages()), [], 'nothing from after the displayed hour is published');
  for (const ask of back.fetch) answer(ask);
  const kept = pager.pages()[8];
  const far = pager.window({ first: 12, count: 2 }, 90);
  assert.deepEqual(far.fetch.map(ask => ask.offset), [12]);
  answer(far.fetch[0]!);
  assert.deepEqual(Object.keys(pager.pages()).map(Number), [8, 12], 'pages beyond one either side of the window are evicted');
  assert.equal(pager.pages()[8], kept, 'a kept page keeps its object, so its revision');
  const advanced = pager.window({ first: 16, count: 2 }, 150);
  assert.deepEqual(advanced.fetch, [{ offset: 16, limit: 4, hour: 90 }], 'scrolling after playback advances asks the new offset at the frozen snapshot hour');
  assert.equal(answer(advanced.fetch[0]!), true, 'a page asked after playback advances settles into the frozen snapshot');
  assert.equal(pager.pages()[16]?.hour, 90);
  const refusedStep = pager.window({ first: 20, count: 2 }, 160);
  assert.deepEqual(refusedStep.fetch, [{ offset: 20, limit: 4, hour: 90 }], 'further scrolling keeps asking at the frozen hour');
  assert.equal(pager.refuse(refusedStep.fetch[0]!, 'Log table withheld'), true, 'a refused page settles and clears its in-flight mark');
  assert.equal('refused' in pager.pages()[20]!, true);
  pager.window({ first: 8, count: 2 }, 160);
  assert.equal(pager.pages()[20], undefined, 'evicting the refused page drops it');
  const retried = pager.window({ first: 20, count: 2 }, 160);
  assert.deepEqual(retried.fetch, [{ offset: 20, limit: 4, hour: 90 }], 'scrolling back asks an evicted refused offset again instead of leaving it in flight');
  pager.window({ first: 0, count: 2 }, 160);
  assert.deepEqual(Object.keys(pager.pages()), [], 'back at the head the snapshot is released');
});

test('a frozen log scrolled far keeps its snapshot header: until the new window answers, no live total, chips or rows show through', () => {
  const pager = new SnapshotPager(4, 4);
  const near = pager.window({ first: 5, count: 2 }, 100);
  const raw = rawReply([{ key: 'p4', hour: 100 }], 40, { offset: '4', counts: [{ key: 'running', total: '3' }] });
  assert.equal(pager.answer(near.fetch[0]!, validateGenericTailReply(raw, 100), raw), true);
  const far = pager.window({ first: 30, count: 2 }, 150);
  assert.deepEqual(far.fetch.map(ask => [ask.offset, ask.hour]), [[28, 100]], 'the far page is asked at the frozen hour');
  const live = rawReply([{ key: 'new', hour: 150 }], 90, { counts: [{ key: 'running', total: '9' }] });
  const source = buildRowSourceFromHead('q', live, undefined, 150, pager.pages())!;
  assert.deepEqual([source.at, source.total, source.facetTotals?.get('running')], [100, 40, 3], 'the frozen snapshot still names the log while the far page loads');
  assert.deepEqual(source.sliceAt(150, 30, 2).map(slot => slot.kind), ['loading', 'loading']);
  const answered = rawReply([{ key: 'p28', hour: 100 }], 40, { offset: '28' });
  pager.answer(far.fetch[0]!, validateGenericTailReply(answered, 100), answered);
  assert.deepEqual(Object.keys(pager.pages()).map(Number), [28], 'once the window answers, the old header page leaves');
});

test('a frozen page refused before any page answered is a dated failure inside the later live head', () => {
  const pager = new SnapshotPager(4, 4);
  const ask = pager.window({ first: 5, count: 2 }, 100).fetch[0]!;
  pager.refuse(ask, 'Log page withheld');
  const live = rawReply([{ key: 'new', hour: 150 }], 9);
  const source = buildRowSourceFromHead('q', live, undefined, 150, pager.pages())!;
  assert.deepEqual([source.at, source.total], [150, 9], 'the live head at the displayed hour, whole');
  const slot = source.sliceAt(150, 5, 1)[0]!;
  assert.deepEqual([slot.kind, slot.kind === 'refused' ? slot.reason : null], ['refused', `Not read at ${formatDateOnly(100)}: Log page withheld`]);
  const sameHour = buildRowSourceFromHead('q', rawReply([{ key: 'old', hour: 100 }], 9), undefined, 100, pager.pages())!;
  const own = sameHour.sliceAt(100, 5, 1)[0]!;
  assert.equal(own.kind === 'refused' ? own.reason : null, 'Log page withheld', 'a refusal at the displayed hour is told as it came');
});

test('each snapshot page crosses to the worker once, however many pages the scroll gathers', () => {
  const crossed: unknown[] = [];
  const sender = new PayloadSender(message => { for (const [, value] of message.put ?? []) crossed.push(value); });
  const p0 = page(100, 0, 4, [{ key: 'a', hour: 100 }], 9);
  const p4 = page(100, 4, 4, [{ key: 'b', hour: 99 }], 9);
  const p8 = page(100, 8, 4, [{ key: 'c', hour: 98 }], 9);
  sender.acquire({ pagedRows: { main: { 0: p0 } } });
  sender.acquire({ pagedRows: { main: { 0: p0, 4: p4 } } });
  sender.acquire({ pagedRows: { main: { 0: p0, 4: p4, 8: p8 } } });
  assert.deepEqual(crossed, [p0, p4, p8], 'one page per message, never the pages gathered so far');
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

  const live = [
    { kind: 'data' as const, ordinal: 0, readAt: 150, row: { key: 'running', hour: 90, label: 'running' } },
    { kind: 'data' as const, ordinal: 1, readAt: 150, row: { key: 'a', hour: 145, label: 'a' } },
  ];
  const placed = buildNavigableSlots(150, live, [{ ...gaps[0]!, before: '2' }, { ...gaps[1]!, before: '2' }], 2);
  assert.deepEqual(placed.map(slot => slot.kind === 'data' ? slot.row.key : slot.kind === 'gap' ? `${slot.firstHour}-${slot.lastHour}@${slot.beforeOrdinal}` : slot.kind), [
    'running',
    'a',
    '130-140@2',
    '100-110@2',
  ], 'a gap the product placed stays at its ordinal, never among the live rows by hour');
});

test('S19: display offsets translate to data ordinals across placed gaps and totalRows stays stable when gaps scroll out of view', () => {
  const sliced: { first: number; count: number }[] = [];
  const gaps = [
    { firstHour: 180, lastHour: 190, reason: 'gap 1', before: '10' },
    { firstHour: 120, lastHour: 130, reason: 'gap 2', before: '50' },
  ];
  const source = {
    identity: 'test:s19',
    total: 400,
    at: 200,
    missing: gaps,
    refused: null,
    sliceAt(reqHour: number, first: number, count: number) {
      sliced.push({ first, count });
      return Array.from({ length: Math.min(count, Math.max(0, 400 - first)) }, (_, i) => ({
        kind: 'data' as const,
        ordinal: first + i,
        readAt: reqHour,
        row: { key: `row-${first + i}`, hour: 100 },
      }));
    },
  };
  const projected = projectPagedSlots(
    source,
    200,
    { first: 300, count: 5 },
    (row, ordinal) => ({ key: row.key, name: `#${ordinal}`, cells: {} }),
  );
  assert.deepEqual(sliced, [{ first: 298, count: 5 }], 'two earlier placed gaps shift display position 300 to data ordinal 298');
  assert.equal(projected.windowOffset, 300, 'layout offset remains the display offset');
  assert.equal(projected.totalRows, 402, 'totalRows includes all placed gaps even when they are above the window');
  assert.deepEqual(projected.gapBefore, [10, 50]);
  assert.deepEqual(projected.rows.map(r => r.key), ['row-298', 'row-299', 'row-300', 'row-301', 'row-302']);

  const adapterWin = tableHostAdapter.rowWindow!(
    { width: 600, height: 180, dpr: 1, scrollLeft: 0, scrollTop: 300 * 36, zoomWindow: null },
    { density: 'normal', barsBehind: 'off', showRank: false, order: [], columns: {}, sort: { column: 'date', direction: 'descending' } },
    { table: { count: 402, totalHeight: 402 * 36, gapBefore: [10, 50] } },
  )!;
  assert.equal(adapterWin.dataFirst, adapterWin.first - 2, 'host adapter translates display scroll offset to data ordinal');
  const pager = new SnapshotPager(256, 256);
  const step = pager.window({ first: 257, count: 5, dataFirst: 255, dataCount: 5 }, 200);
  assert.deepEqual(step.fetch, [{ offset: 0, limit: 256, hour: 200 }, { offset: 256, limit: 256, hour: 200 }], 'pager asks data-ordinal pages across the boundary shifted by earlier gaps');
});

test('a dated tail follows its selected categories and group', () => {
  const tail = tailFollowParams({
    product: 'event-feed-tail',
    branchId: '1',
    from: gatedHour(100),
    limit: 256,
    ask: { categories: ['politics'], group: { type: 'faction', id: '42', membership: 'hour' } },
  });
  assert.deepEqual(tail, { panel: 'event-feed-tail', branchId: '1', from: 100, limit: 256, categories: ['politics'], group: { type: 'faction', id: '42', membership: 'hour' } },
    'a tail panel carries its real follow parameters and the group, never a member list');
});

test('S3: tail decoder turns per-row decode errors into refused slots with their ordinal and reason on both head and snapshot pages', () => {
  const validate = (raw: unknown, readAt?: number) => validateRowTailReply<Row>(raw, row => {
    if (typeof row.label !== 'string') throw new Error('Row payload is malformed.');
    return { key: row.key as string, hour: row.hour as number, label: row.label };
  }, readAt);

  const head = rawReply([
    { key: 'bad0', hour: 120 } as unknown as Row,
    { key: 'ok1', hour: 100, label: 'good' } as unknown as Row,
  ], 2, { missing: [{ firstHour: 110, lastHour: 115, reason: 'Collector detached', before: '1' }] });
  const source = buildRowSourceFromHead('q:head', head, undefined, 130, undefined, validate)!;
  assert.deepEqual(source.sliceAt(130, 0, 2), [
    { kind: 'refused', ordinal: 0, readAt: 130, hour: 120, reason: 'Row payload is malformed.' },
    { kind: 'data', ordinal: 1, readAt: 130, row: { key: 'ok1', hour: 100, label: 'good' } },
  ]);
  const projected = projectPagedSlots(source, 130, { first: 0, count: 10 }, row => ({ key: row.key, name: row.label, cells: {} }));
  assert.deepEqual(projected.slots.map(slot => (slot.kind === 'data' ? ['data', slot.row.key] : slot.kind === 'refused' ? ['refused', slot.key, slot.reason] : slot.kind === 'gap' ? ['gap', slot.reason] : [slot.kind])), [
    ['refused', 'refused:0', 'Row payload is malformed.'],
    ['gap', 'Collector detached'],
    ['data', 'ok1'],
  ], 'a refused row keeps its ordinal alongside placed gaps');

  const rawPage = rawReply([
    { key: 'p4', hour: 95, label: 'four' } as unknown as Row,
    { key: 'p5', hour: 94 } as unknown as Row,
  ], 6, { offset: '4' });
  const pages: SnapshotPages<Row> = {
    4: { hour: 100, offset: 4, limit: 4, raw: rawPage, reply: validate(rawPage, 100) },
  };
  const frozen = buildRowSourceFromHead<Row>('q:frozen', undefined, undefined, 130, pages, validate)!;
  assert.deepEqual(frozen.sliceAt(130, 4, 2), [
    { kind: 'data', ordinal: 4, readAt: 130, row: { key: 'p4', hour: 95, label: 'four' } },
    { kind: 'refused', ordinal: 5, readAt: 130, hour: 94, reason: 'Row payload is malformed.' },
  ]);
});
