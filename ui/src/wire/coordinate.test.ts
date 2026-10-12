import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeReply, NO_INDEX, UNKNOWN_TOTAL, WireError } from './runtime.ts';
import type { Descriptor, Extent, ShapeContract } from './runtime.ts';
import { bytesOf, encodeSynthetic, u64 } from './test-support.ts';

function descriptor(nameId: number, logicalType: number, extra: Partial<Descriptor> = {}): Descriptor {
  return { nameId, logicalType, encoding: 0, stateLayers: 0, paramA: 0, paramB: 0,
    dictionaryRef: NO_INDEX, childFirst: NO_INDEX, childCount: 0, dataBuffer: NO_INDEX,
    length: UNKNOWN_TOTAL, auxBuffer: NO_INDEX, stateBuffer: NO_INDEX, stateWidths: 0,
    gateColumn: 0xffff, gateValue: 0, refusalRef: NO_INDEX, ...extra };
}
function contract(id: number, names: string[], descriptors: Descriptor[], topLevel: number[],
  replyKind = 4, unitKinds = [0], bounds: ShapeContract['bounds'] = 'forbidden'): ShapeContract {
  return { id, version: 1, name: 'synthetic.coordinates', replyKind, unitKinds, names,
    columns: descriptors.map(d => ({ descriptor: d })), topLevel, bounds };
}
function extentBytes(extents: Extent[]): Uint8Array {
  const bytes = new Uint8Array(extents.length * 40), view = new DataView(bytes.buffer);
  extents.forEach((extent, index) => {
    const at = index * 40;
    view.setUint32(at, extent.column, true); view.setUint8(at + 4, extent.coordinate);
    view.setUint8(at + 5, Number(extent.final));
    for (const [offset, value] of [[8, extent.index], [16, extent.first], [24, extent.end], [32, extent.total]] as const)
      view.setBigUint64(at + offset, value, true);
  });
  return bytes;
}
function dictionaryBytes(firstEntry: bigint, entryCount: bigint): Uint8Array {
  const bytes = new Uint8Array(88), view = new DataView(bytes.buffer);
  bytes.fill(0x11, 0, 32); bytes.fill(0x22, 32, 64);
  view.setBigUint64(64, firstEntry, true); view.setBigUint64(72, entryCount, true);
  view.setUint32(80, 0, true);
  return bytes;
}

test('inverse offsets retain original battle, run, side, and country positions', () => {
  const descriptors = [
    descriptor(0, 14, { paramA: 64, childFirst: 1, childCount: 1, auxBuffer: 0 }),
    descriptor(1, 15, { childFirst: 2, childCount: 1, length: 4n }),
    descriptor(2, 14, { paramA: 64, childFirst: 3, childCount: 1, length: 4n, auxBuffer: 1 }),
    descriptor(3, 14, { paramA: 64, childFirst: 4, childCount: 1, length: 8n, auxBuffer: 2 }),
    descriptor(4, 2, { paramA: 64, length: 4n, dataBuffer: 3 }),
  ];
  const shape = contract(99001, ['runs', 'runs.$item', 'runs.$item.countries',
    'runs.$item.countries.$item', 'runs.$item.countries.$item.$item'], descriptors, [0]);
  const extents: Extent[] = [
    { column: 0, coordinate: 1, final: false, index: 0n, first: 10n, end: 12n, total: 30n },
    { column: 0, coordinate: 1, final: false, index: 1n, first: 20n, end: 22n, total: 25n },
    { column: 3, coordinate: 1, final: false, index: 5n, first: 8n, end: 12n, total: 15n },
  ];
  const buffer = encodeSynthetic({ shapeId: shape.id, replyKind: 4, descriptors, topLevel: [0],
    buffers: [u64([0n, 2n, 4n]), u64([0n, 2n, 4n, 6n, 8n]),
      u64([0n, 0n, 0n, 0n, 0n, 0n, 4n, 4n, 4n]), u64([1n, 2n, 3n, 4n])],
    rowCount: 2n, unitStart: 40n, totalCount: 100n,
    extraSections: [{ kind: 8, count: extents.length, bytes: extentBytes(extents) }] });
  const reply = decodeReply(buffer, shape);
  assert.deepEqual(reply.originalIdentity(1, 2), { row: 41n, elements: [20n] });
  assert.deepEqual(reply.originalIdentity(3, 5), { row: 41n, elements: [20n, 1n] });
  assert.deepEqual(reply.originalIdentity(4, 0), { row: 41n, elements: [20n, 1n, 8n] });
  assert.deepEqual(reply.originalIdentity(4, 3), { row: 41n, elements: [20n, 1n, 11n] });
  assert.deepEqual(reply.originalIdentity(0, 1), { row: 41n, elements: [] });
});

test('century continuations keep one original row and u64 run coordinates', () => {
  const centuryHours = 100n * 365n * 24n, centuryRuns = centuryHours * 5000n;
  assert.equal(centuryRuns, 4_380_000_000n);
  assert.ok(centuryRuns > 0xffff_ffffn);
  const descriptors = [
    descriptor(0, 14, { paramA: 64, childFirst: 1, childCount: 1, auxBuffer: 0 }),
    descriptor(1, 2, { paramA: 64, length: 2n, dataBuffer: 1 }),
  ];
  const shape = contract(99002, ['runs', 'runs.$item'], descriptors, [0], 1, [1], 'required');
  const bounds = { kind: 0, start: centuryHours - 2n, end: centuryHours };
  for (const page of [0n, 1n]) {
    const first: bigint = centuryRuns + page * 2n;
    const extent: Extent = { column: 0, coordinate: 1, final: page === 1n, index: 0n,
      first, end: first + 2n, total: centuryRuns + 4n };
    const buffer = encodeSynthetic({ shapeId: shape.id, replyKind: 1, unitKind: 1,
      descriptors, topLevel: [0], buffers: [u64([0n, 2n]), u64([7n, 8n])],
      rowCount: 1n, unitStart: centuryRuns, totalCount: centuryRuns + 1n, bounds,
      extraSections: [{ kind: 8, count: 1, bytes: extentBytes([extent]) }] });
    const reply = decodeReply(buffer, shape, { bounds });
    assert.deepEqual(reply.bounds, bounds);
    assert.deepEqual(reply.originalIdentity(1, 0), { row: centuryRuns, elements: [first] });
    assert.deepEqual(reply.originalIdentity(1, 1), { row: centuryRuns, elements: [first + 1n] });
    const shortExtent = { ...extent, first: page * 2n, end: page * 2n + 2n, total: 4n };
    const shortBuffer = encodeSynthetic({ shapeId: shape.id, replyKind: 1, unitKind: 1,
      descriptors, topLevel: [0], buffers: [u64([0n, 2n]), u64([7n, 8n])],
      rowCount: 1n, unitStart: 0n, totalCount: 1n, bounds: { kind: 0, start: 0n, end: 2n },
      extraSections: [{ kind: 8, count: 1, bytes: extentBytes([shortExtent]) }] });
    assert.equal(buffer.byteLength, shortBuffer.byteLength,
      'The same requested unit has the same size in a short campaign and at century coordinates.');
  }
});

test('chart run hours use direct signed i32 views and keep an i64 exclusive boundary', () => {
  const descriptors = [
    descriptor(0, 2, { paramA: 64, dataBuffer: 0 }),
    descriptor(1, 14, { paramA: 64, childFirst: 3, childCount: 1, auxBuffer: 1 }),
    descriptor(2, 10, { auxBuffer: 2, dataBuffer: 3 }),
    descriptor(3, 15, { childFirst: 4, childCount: 4, length: 2n }),
    descriptor(4, 3, { paramA: 32, dataBuffer: 4, length: 2n }),
    descriptor(5, 3, { paramA: 32, dataBuffer: 5, length: 2n }),
    descriptor(6, 7, { paramA: 64, dataBuffer: 6, length: 2n }),
    descriptor(7, 1, { encoding: 1, paramB: 1, dataBuffer: 7, length: 2n }),
  ];
  const shape = contract(99003, ['first_run', 'runs', 'directory', 'runs.$item',
    'runs.$item.first', 'runs.$item.last', 'runs.$item.value', 'runs.$item.break_before'],
  descriptors, [0, 1, 2], 1, [1], 'required');
  const bounds = { kind: 0, start: -2_147_483_648n, end: 2_147_483_648n };
  for (const directory of [new Uint8Array([0x48, 0x34]), new Uint8Array()]) {
    const buffer = encodeSynthetic({ shapeId: shape.id, replyKind: 1, unitKind: 1, descriptors,
      topLevel: [0, 1, 2], rowCount: 1n, bounds,
      buffers: [u64([4_380_000_000n]), u64([0n, 2n]), u64([0n, BigInt(directory.length)]), directory,
        bytesOf(new Int32Array([-2_147_483_648, 2_147_483_647])),
        bytesOf(new Int32Array([-2_147_483_648, 2_147_483_647])),
        u64([0x7ff8_0000_0000_1234n, 0x8000_0000_0000_0000n]), new Uint8Array([1])] });
    const reply = decodeReply(buffer, shape, { bounds });
    for (const index of [4, 5]) {
      const data = reply.columns[index].data;
      assert.ok(data instanceof Int32Array); assert.equal(data.buffer, buffer);
      assert.deepEqual([...data], [-2_147_483_648, 2_147_483_647]);
    }
    assert.equal(reply.columns[0].value(0), 4_380_000_000n);
    assert.deepEqual(reply.columns[2].value(0), directory);
    const values = reply.columns[6].data as Float64Array;
    assert.equal(values.buffer, buffer);
    assert.equal(new DataView(buffer).getBigUint64(values.byteOffset, true), 0x7ff8_0000_0000_1234n);
    assert.ok(Object.is(values[1], -0));
    assert.equal(reply.columns[7].value(0), 1);
    assert.equal(reply.columns[7].value(1), 0);
  }
});

test('dictionary codes and canonical identities retain u64 widths beyond century run counts', () => {
  const firstEntry = (1n << 40n) + 7n, entryCount = (1n << 32n) + 3n;
  const descriptors = [descriptor(0, 12, { encoding: 2, paramA: 64, dictionaryRef: 0, dataBuffer: 0 })];
  const shape = contract(99004, ['key'], descriptors, [0]);
  const write = (first: bigint) => encodeSynthetic({ shapeId: shape.id, replyKind: 4, descriptors,
    topLevel: [0], buffers: [u64([0n, (1n << 32n) + 1n])], rowCount: 2n,
    extraSections: [{ kind: 4, count: 1, bytes: dictionaryBytes(first, entryCount) }] });
  const buffer = write(firstEntry), reply = decodeReply(buffer, shape);
  assert.ok(reply.columns[0].data instanceof BigUint64Array);
  assert.equal(reply.columns[0].data.buffer, buffer);
  assert.equal(reply.columns[0].value(1), 4_294_967_297n);
  assert.equal(reply.dictionaries[0].firstEntry + (reply.columns[0].value(1) as bigint),
    firstEntry + 4_294_967_297n);
  assert.throws(() => decodeReply(write(UNKNOWN_TOTAL - 1n), shape),
    (error: unknown) => error instanceof WireError && error.code === 'dictionary-overflow');
});
