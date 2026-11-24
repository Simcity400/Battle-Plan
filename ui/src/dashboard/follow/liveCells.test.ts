import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { CELL } from './message.ts';
import type { FollowedRows } from './message';
import { CellGroup, ZERO_DIVISOR, liveSection, repliedCells, sectionGroups, sectionValues } from './liveCells.ts';
import type { LiveSection } from './liveCells.ts';

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


test('CellGroup and sectionValues preserve unsafe 64-bit integers through raw()', () => {
  const buffer = new ArrayBuffer(64);
  const i64 = new BigInt64Array(buffer, 0, 1);
  i64[0] = 9007199254740993n;
  const u8 = new Uint8Array(buffer);
  u8[8] = CELL.value;
  const u32 = new Uint32Array(buffer, 16, 1);
  u32[0] = 0xffff_ffff;
  const answer: FollowedRows = {
    kind: 'rows',
    rows: 1,
    columns: 1,
    views: {
      u8,
      u16: new Uint16Array(buffer),
      u32: new Uint32Array(buffer),
      i32: new Int32Array(buffer),
      i64: new BigInt64Array(buffer),
      u64: new BigUint64Array(buffer),
    },
    keys: 0,
    values: 0,
    hours: -1,
    states: 8,
    reasons: 4,
    names: 4,
    canonical: true,
    ends: new Uint32Array(1),
    text: 0,
    decoded: [],
  };
  const group = new CellGroup();
  group.land(answer);
  const liveSectionSpec: LiveSection = {
    groups: [{ table: 70, columns: [0], keys: ['k'], follow: 0x8000_0001 }],
    rows: ['k'],
    kinds: [{ id: 'total', op: 'value', scale: 1 }],
    cells: [0, 0, 1, 0, 0, 0],
  };
  const values = sectionValues(liveSectionSpec, sectionGroups(liveSectionSpec, () => group));
  assert.equal(values.raw?.(liveKey('k', 'total')), 9007199254740993n);
  assert.equal(values.get(liveKey('k', 'total')), Number(9007199254740993n));
});

test('repliedCells preserves unsafe 64-bit integers from ordinary JSON head replies via rawValues', () => {
  const jsonText = JSON.stringify({
    leaves: ['k', 'm'],
    warnings: [],
    live: {
      groups: [{
        table: 70,
        columns: [0],
        keys: ['k', 'm'],
        values: [Number(9007199254740993n), 'Not recorded at this hour'],
        rawValues: ['9007199254740993', null],
      }],
      rows: ['k', 'm'],
      kinds: [{ id: 'total', op: 'value', scale: 1 }],
      cells: [0, 0, 1, 0, 0, 0, 1, 0, 1, 0, 1, 0],
    },
  });
  const values = repliedCells(JSON.parse(jsonText)).read();
  assert.equal(values.raw?.(liveKey('k', 'total')), 9007199254740993n);
  assert.equal(values.get(liveKey('k', 'total')), Number(9007199254740993n));
  assert.equal(values.raw?.(liveKey('m', 'total')), undefined);
  assert.equal(values.missing(liveKey('m', 'total')), 'Not recorded at this hour');

  assert.throws(() => liveSection({
    live: {
      groups: [{ table: 70, columns: [0], keys: ['k'], values: [1], rawValues: ['1.5'] }],
      rows: ['k'],
      kinds: [{ id: 'total', op: 'value', scale: 1 }],
      cells: [0, 0, 1, 0, 0, 0],
    },
  }), /malformed/);
  assert.throws(() => liveSection({
    live: {
      groups: [{ table: 70, columns: [0], keys: ['k'], values: ['Not recorded'], rawValues: ['1'] }],
      rows: ['k'],
      kinds: [{ id: 'total', op: 'value', scale: 1 }],
      cells: [0, 0, 1, 0, 0, 0],
    },
  }), /malformed/);
});

test('liveSection ignores occurrence tail live-count strings while still rejecting malformed live payloads', () => {
  assert.equal(liveSection({ branchId: '1', total: '12', offset: '0', live: '3', rows: [] }), null);
  assert.throws(() => liveSection({ live: 'bad' }), /malformed/);
  assert.throws(() => liveSection({ live: '3' }), /malformed/);
});
