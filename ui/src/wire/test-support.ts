/** Synthetic Appendix C bytes for runtime tests and benchmarks. Production writes in Rust. */
import { xxh3 } from './checksum.ts';
import type { Descriptor, ShapeContract } from './runtime.ts';

export const NONE = 0xffff_ffff;
export const NO_TOTAL = 0xffff_ffff_ffff_ffffn;
export const align8 = (n: number): number => Math.ceil(n / 8) * 8;

export interface SyntheticDescriptor {
  nameId: number; logicalType: number; encoding?: number; stateLayers?: number;
  paramA?: number; paramB?: number; dictionaryRef?: number; childFirst?: number;
  childCount?: number; dataBuffer?: number; length?: bigint; auxBuffer?: number;
  stateBuffer?: number; stateWidths?: number; gateColumn?: number; gateValue?: number;
  refusalRef?: number;
}
export interface SyntheticReply {
  shapeId: number; shapeVersion?: number; replyKind: number; unitKind?: number;
  descriptors: SyntheticDescriptor[]; topLevel: number[]; buffers: Uint8Array[];
  rowCount: bigint; unitStart?: bigint; totalCount?: bigint; diagnosticTotal?: bigint;
  bounds?: { kind: number; start: bigint; end: bigint };
  extraSections?: {kind: number; count: number; bytes: Uint8Array}[];
  flags?: number;
}

export function bytesOf(array: ArrayBufferView): Uint8Array {
  return new Uint8Array(array.buffer, array.byteOffset, array.byteLength);
}
export function u64(values: bigint[]): Uint8Array { return bytesOf(new BigUint64Array(values)); }

/** Deliberately synthetic contracts; public product contracts are Rust-generated. */
export function syntheticShape(reply: SyntheticReply, names: string[]): ShapeContract {
  return {
    id: reply.shapeId, version: reply.shapeVersion ?? 1, name: 'synthetic-wire-test',
    replyKind: reply.replyKind, unitKinds: [reply.unitKind ?? 0], names,
    topLevel: reply.topLevel, bounds: reply.bounds ? 'required' : 'none',
    columns: reply.descriptors.map(column => ({ descriptor: {
      nameId: column.nameId, logicalType: column.logicalType, encoding: column.encoding ?? 0,
      stateLayers: column.stateLayers ?? 0, paramA: column.paramA ?? 0, paramB: column.paramB ?? 0,
      dictionaryRef: column.dictionaryRef ?? NONE, childFirst: column.childFirst ?? NONE,
      childCount: column.childCount ?? 0, dataBuffer: column.dataBuffer ?? NONE,
      length: column.length ?? NO_TOTAL, auxBuffer: column.auxBuffer ?? NONE,
      stateBuffer: column.stateBuffer ?? NONE, stateWidths: column.stateWidths ?? 0,
      gateColumn: column.gateColumn ?? 0xffff, gateValue: column.gateValue ?? 0,
      refusalRef: column.refusalRef ?? NONE,
    } satisfies Descriptor })),
  };
}

export function encodeSynthetic(reply: SyntheticReply): ArrayBuffer {
  const descriptors = new Uint8Array(reply.descriptors.length * 64);
  const d = new DataView(descriptors.buffer);
  reply.descriptors.forEach((column, index) => {
    const p = index * 64;
    d.setUint32(p, column.nameId, true); d.setUint16(p + 4, column.logicalType, true);
    d.setUint8(p + 6, column.encoding ?? 0); d.setUint8(p + 7, column.stateLayers ?? 0);
    d.setUint32(p + 8, column.paramA ?? 0, true); d.setUint32(p + 12, column.paramB ?? 0, true);
    d.setUint32(p + 16, column.dictionaryRef ?? NONE, true);
    d.setUint32(p + 20, column.childFirst ?? NONE, true);
    d.setUint32(p + 24, column.childCount ?? 0, true);
    d.setUint32(p + 28, column.dataBuffer ?? NONE, true);
    d.setBigUint64(p + 32, column.length ?? NO_TOTAL, true);
    d.setUint32(p + 40, column.auxBuffer ?? NONE, true);
    d.setUint32(p + 44, column.stateBuffer ?? NONE, true);
    d.setUint16(p + 48, column.stateWidths ?? 0, true);
    d.setUint16(p + 50, column.gateColumn ?? 0xffff, true);
    d.setUint32(p + 52, column.gateValue ?? 0, true);
    d.setUint32(p + 56, column.refusalRef ?? NONE, true);
  });
  let bufferLength = reply.buffers.length * 16;
  const bufferOffsets = reply.buffers.map(bytes => {
    bufferLength = align8(bufferLength);
    const offset = bufferLength; bufferLength += bytes.length;
    return offset;
  });
  const buffers = new Uint8Array(bufferLength);
  const table = new DataView(buffers.buffer);
  reply.buffers.forEach((bytes, i) => {
    table.setBigUint64(i * 16, BigInt(bufferOffsets[i]), true);
    table.setBigUint64(i * 16 + 8, BigInt(bytes.length), true);
    buffers.set(bytes, bufferOffsets[i]);
  });
  const sections = [
    {kind: 1, count: reply.descriptors.length, bytes: descriptors},
    {kind: 2, count: reply.topLevel.length, bytes: bytesOf(new Uint32Array(reply.topLevel))},
    {kind: 3, count: reply.buffers.length, bytes: buffers},
  ];
  if (reply.bounds) {
    const bytes = new Uint8Array(24); const view = new DataView(bytes.buffer);
    view.setUint32(0, reply.bounds.kind, true);
    view.setBigInt64(8, reply.bounds.start, true); view.setBigInt64(16, reply.bounds.end, true);
    sections.push({kind: 9, count: 1, bytes});
  }
  sections.push(...reply.extraSections ?? []);
  let totalLength = 128 + sections.length * 24;
  const offsets = sections.map(section => {
    totalLength = align8(totalLength);
    const offset = totalLength; totalLength += section.bytes.length;
    return offset;
  });
  const result = new ArrayBuffer(totalLength);
  const view = new DataView(result); const bytes = new Uint8Array(result);
  bytes.set([72, 52, 82, 81]); view.setUint16(4, 1, true);
  view.setUint16(6, reply.flags ?? 1, true);
  view.setBigUint64(8, 1n, true); // query
  view.setBigUint64(32, 1n, true); view.setBigUint64(40, 1n, true);
  view.setUint32(48, reply.replyKind, true); view.setUint32(52, reply.shapeId, true);
  view.setUint32(56, reply.shapeVersion ?? 1, true); view.setUint32(60, sections.length, true);
  view.setUint32(64, reply.unitKind ?? 0, true);
  const first = reply.unitStart ?? 0n;
  view.setBigUint64(72, first, true); view.setBigUint64(80, first + reply.rowCount, true);
  view.setBigUint64(88, reply.rowCount, true);
  view.setBigUint64(96, reply.totalCount ?? first + reply.rowCount, true);
  view.setBigUint64(104, reply.diagnosticTotal ?? 0n, true);
  view.setBigUint64(112, BigInt(totalLength), true);
  sections.forEach((section, i) => {
    const p = 128 + i * 24;
    view.setUint32(p, section.kind, true); view.setUint32(p + 4, section.count, true);
    view.setBigUint64(p + 8, BigInt(offsets[i]), true);
    view.setBigUint64(p + 16, BigInt(section.bytes.length), true);
    bytes.set(section.bytes, offsets[i]);
  });
  refreshChecksum(result);
  return result;
}

export function refreshChecksum(buffer: ArrayBuffer): void {
  new DataView(buffer).setBigUint64(120, xxh3(new Uint8Array(buffer, 128)), true);
}
