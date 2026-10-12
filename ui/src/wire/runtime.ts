import { xxh3 } from './checksum.ts';

export const NO_INDEX = 0xffff_ffff;
export const UNKNOWN_TOTAL = 0xffff_ffff_ffff_ffffn;
export const WIRE_SCHEMA_VERSION = 1;
export const DIAGNOSTICS_KIND = 10;
const NO_GATE = 0xffff;
const littleEndian = new Uint8Array(new Uint16Array([1]).buffer)[0] === 1;
const utf8 = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

export class WireError extends Error {
  readonly code: string;
  readonly queryId?: bigint;
  constructor(code: string, detail = '', queryId?: bigint) {
    super(detail ? `${code}: ${detail}` : code);
    this.name = code; this.code = code; this.queryId = queryId;
  }
}
function requireWire(condition: unknown, code: string, detail = ''): asserts condition {
  if (!condition) throw new WireError(code, detail);
}
/** Converts a local coordinate only after its actual byte/element bound was checked. */
function local(n: bigint, maximum: number, error = 'size-overflow'): number {
  requireWire(n >= 0n && n <= BigInt(maximum), error);
  return Number(n);
}
function checkedAdd(a: bigint, b: bigint, error = 'size-overflow'): bigint {
  const result = a + b;
  requireWire(result <= UNKNOWN_TOTAL, error);
  return result;
}

export interface Descriptor {
  nameId: number; logicalType: number; encoding: number; stateLayers: number;
  paramA: number; paramB: number; dictionaryRef: number; childFirst: number;
  childCount: number; dataBuffer: number; length: bigint; auxBuffer: number;
  stateBuffer: number; stateWidths: number; gateColumn: number; gateValue: number;
  refusalRef: number;
}
export interface ColumnSpec {
  descriptor: Pick<Descriptor, 'nameId' | 'logicalType'> & Partial<Descriptor>;
  staticEnum?: readonly string[];
  fixedListLength?: bigint;
  dictionaryKind?: number;
  refusalKind?: number;
}
// Shared shape defaults are applied at comparison, without copying generated records.
const SHAPE_DESCRIPTOR_DEFAULTS = { encoding:0, stateLayers:0, paramA:0, paramB:0,
  childFirst:NO_INDEX, childCount:0, gateColumn:NO_GATE, gateValue:0 };
const SHAPE_DESCRIPTOR_KEYS = ['nameId','logicalType','encoding','stateLayers','paramA','paramB',
  'childFirst','childCount','gateColumn','gateValue'] as const;
export interface ShapeContract {
  id: number; version: number; name: string; replyKind: number;
  unitKinds: readonly number[]; names: readonly string[];
  columns: readonly ColumnSpec[]; topLevel: readonly number[];
  bounds: 'none' | 'forbidden' | 'required' | 'optional';
  diagnostics?: ShapeContract;
}
export interface Bounds { kind: number; start: bigint; end: bigint }
export interface ExpectedReply {
  queryId?: bigint; campaignId?: Uint8Array; revision?: bigint; generation?: bigint;
  /** Omission means the request has no bounds; never inferred from received bytes. */
  bounds?: Bounds;
  diagnostics?: ShapeContract;
}
export interface ReplyHeader {
  flags: number; queryId: bigint; campaignId: Uint8Array; revision: bigint; generation: bigint;
  replyKind: number; shapeId: number; shapeVersion: number; sectionCount: number; unitKind: number;
  unitStart: bigint; unitEnd: bigint; rowCount: bigint; totalCount: bigint;
  diagnosticTotal: bigint; totalLength: bigint; checksum: bigint;
}
export interface DictionaryReference {
  identity: Uint8Array; shardIdentity: Uint8Array; firstEntry: bigint;
  entryCount: bigint; kind: number; flags: number;
}
export interface Extent {
  column: number; coordinate: number; final: boolean; index: bigint;
  first: bigint; end: bigint; total: bigint;
}
export type NumericArray = Uint8Array | Int8Array | Uint16Array | Int16Array | Uint32Array |
  Int32Array | BigUint64Array | BigInt64Array | Float32Array | Float64Array;
type Scalar = number | bigint;
export interface ScalarView {
  readonly length: number;
  readonly data?: NumericArray;
  readonly runEnds?: BigUint64Array;
  readonly runValues?: ScalarView;
  get(index: number | bigint): Scalar;
}
export interface ValueState {
  active: boolean;
  /** Absent layers are null. Codes after the first absent layer are not interpreted. */
  layers: readonly (bigint | null)[];
  present: boolean;
}
export interface ColumnView {
  readonly descriptor: Descriptor; readonly length: number; readonly logicalLength: bigint;
  readonly data?: NumericArray; readonly scalar?: ScalarView;
  readonly lo?: BigUint64Array; readonly hi?: BigUint64Array;
  readonly offsets?: BigUint64Array; readonly bytes?: Uint8Array;
  readonly children: readonly number[]; readonly states: readonly (ScalarView | undefined)[];
  state(index: number | bigint): ValueState;
  /** Scalar values stay exact. Bytes/text are views; decoding text is an explicit display operation. */
  value(index: number | bigint): Scalar | Uint8Array | { first: bigint; end: bigint } | undefined;
}
export interface OriginalIdentity { row: bigint; elements: readonly bigint[] }
export interface DecodedReply {
  readonly buffer: ArrayBuffer; readonly header: ReplyHeader; readonly shape: ShapeContract;
  readonly columns: readonly ColumnView[]; readonly topLevel: readonly ColumnView[];
  readonly dictionaries: readonly DictionaryReference[]; readonly extents: readonly Extent[];
  readonly bounds?: Bounds; readonly continuation?: Uint8Array;
  readonly diagnosticContinuation?: Uint8Array; readonly diagnostics?: DecodedReply;
  originalIdentity(column: number, index: number | bigint): OriginalIdentity;
}
interface Section { kind: number; count: number; offset: number; length: number }
interface Slice { offset: number; length: number }
interface Parent { column: number; list: boolean }
function elementIndex(index: number | bigint, count: bigint): bigint {
  requireWire(typeof index === 'bigint' || Number.isSafeInteger(index), 'descriptor-index');
  const coordinate = BigInt(index);
  requireWire(coordinate >= 0n && coordinate < count, 'descriptor-index'); return coordinate;
}

function zeroPadding(bytes: Uint8Array, first: number, end: number): void {
  for (let i = first; i < end; ++i) requireWire(bytes[i] === 0, 'nonzero-padding');
}
function parseDescriptor(view: DataView, offset: number): Descriptor {
  requireWire(view.getUint32(offset + 60, true) === 0, 'descriptor-reserved');
  return {
    nameId: view.getUint32(offset, true), logicalType: view.getUint16(offset + 4, true),
    encoding: view.getUint8(offset + 6), stateLayers: view.getUint8(offset + 7),
    paramA: view.getUint32(offset + 8, true), paramB: view.getUint32(offset + 12, true),
    dictionaryRef: view.getUint32(offset + 16, true), childFirst: view.getUint32(offset + 20, true),
    childCount: view.getUint32(offset + 24, true), dataBuffer: view.getUint32(offset + 28, true),
    length: view.getBigUint64(offset + 32, true), auxBuffer: view.getUint32(offset + 40, true),
    stateBuffer: view.getUint32(offset + 44, true), stateWidths: view.getUint16(offset + 48, true),
    gateColumn: view.getUint16(offset + 50, true), gateValue: view.getUint32(offset + 52, true),
    refusalRef: view.getUint32(offset + 56, true),
  };
}
function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((n, i) => n === b[i]);
}
function integerWidth(width: number): void {
  requireWire(width === 8 || width === 16 || width === 32 || width === 64, 'invalid-type-parameter');
}
function numericView(buffer: ArrayBuffer, offset: number, length: number, width: number,
  signed: boolean, float = false): NumericArray {
  if (float) return width === 32 ? new Float32Array(buffer, offset, length) : new Float64Array(buffer, offset, length);
  switch (width) {
    case 8: return signed ? new Int8Array(buffer, offset, length) : new Uint8Array(buffer, offset, length);
    case 16: return signed ? new Int16Array(buffer, offset, length) : new Uint16Array(buffer, offset, length);
    case 32: return signed ? new Int32Array(buffer, offset, length) : new Uint32Array(buffer, offset, length);
    default: return signed ? new BigInt64Array(buffer, offset, length) : new BigUint64Array(buffer, offset, length);
  }
}
function plainScalar(buffer: ArrayBuffer, slice: Slice, count: bigint, width: number,
  signed: boolean, float = false): ScalarView {
  requireWire(count * BigInt(width / 8) === BigInt(slice.length), 'buffer-length');
  const n = local(count, slice.length / (width / 8));
  const data = numericView(buffer, slice.offset, n, width, signed, float);
  return { length: n, data, get(index) {
    if (typeof index === 'number') {
      requireWire(Number.isSafeInteger(index) && index >= 0 && index < n, 'descriptor-index');
      return data[index];
    }
    return data[local(elementIndex(index, count), n - 1, 'descriptor-index')];
  } };
}
function packedScalar(buffer: ArrayBuffer, slice: Slice, count: bigint, width: number,
  signed: boolean): ScalarView {
  requireWire(width >= 1 && width <= 64, 'invalid-type-parameter');
  requireWire((count * BigInt(width) + 7n) / 8n === BigInt(slice.length), 'buffer-length');
  const n = local(count, slice.length * 8);
  const bytes = new Uint8Array(buffer, slice.offset, slice.length);
  return { length: n, data: bytes, get(index) {
    const coordinate = typeof index === 'number' ? index : local(elementIndex(index, count), n - 1, 'descriptor-index');
    requireWire(Number.isSafeInteger(coordinate) && coordinate >= 0 && coordinate < n, 'descriptor-index');
    if (width === 1) {
      const bit = (bytes[Math.floor(coordinate / 8)] >>> (coordinate & 7)) & 1;
      return signed && bit ? -1 : bit;
    }
    let bit = coordinate * width, remaining = width, shift = 0n, value = 0n;
    while (remaining > 0) {
      const take = Math.min(remaining, 8 - bit % 8);
      value |= BigInt((bytes[Math.floor(bit / 8)] >>> (bit % 8)) & ((1 << take) - 1)) << shift;
      remaining -= take; shift += BigInt(take); bit += take;
    }
    if (signed) value = BigInt.asIntN(width, value);
    return width <= 32 ? Number(value) : value;
  } };
}
function runScalar(values: ScalarView, runs: Uint32Array, count: bigint): ScalarView {
  requireWire(values.length === runs.length, 'run-length');
  let sum = 0n;
  const ends = new BigUint64Array(runs.length);
  for (let i = 0; i < runs.length; ++i) {
    requireWire(runs[i] > 0, 'run-length'); sum = checkedAdd(sum, BigInt(runs[i]), 'run-length'); ends[i] = sum;
  }
  requireWire(sum === count, 'run-length');
  return { get length() { return local(count, Number.MAX_SAFE_INTEGER); }, data: values.data, runEnds:ends, runValues:values, get(index) {
    const coordinate = elementIndex(index, count);
    let first = 0, last = ends.length;
    while (first < last) { const middle = Math.floor((first + last) / 2);
      if (ends[middle] <= coordinate) first = middle + 1; else last = middle; }
    return values.get(first);
  } };
}
function listParent(offsets: BigUint64Array, index: number | bigint): number {
  const position = BigInt(index);
  let low = 0, high = offsets.length - 1;
  while (low < high) {
    const middle = Math.floor((low + high + 1) / 2);
    if (offsets[middle] <= position) low = middle; else high = middle - 1;
  }
  requireWire(low + 1 < offsets.length && position < offsets[low + 1], 'offsets-bounds');
  return low;
}

/** Validate UTF-8 without decoding or copying, allowing only an extent's open boundaries. */
function validateText(bytes: Uint8Array, first: number, end: number, extent?: Extent): void {
  let i = first;
  if (extent && extent.first > 0n) {
    while (i < end && (bytes[i] & 0xc0) === 0x80) i++;
    requireWire(i-first <= 3, 'invalid-utf8');
  }
  while (i < end) {
    const lead = bytes[i++];
    if (lead < 0x80) continue;
    const width = lead >= 0xc2 && lead <= 0xdf ? 2 : lead >= 0xe0 && lead <= 0xef ? 3 :
      lead >= 0xf0 && lead <= 0xf4 ? 4 : 0;
    requireWire(width !== 0, 'invalid-utf8');
    for (let part = 1; part < width; part++) {
      if (i === end) {
        requireWire(extent && !extent.final && (extent.total === UNKNOWN_TOTAL ||
          extent.total - extent.end >= BigInt(width-part)), 'invalid-utf8');
        return;
      }
      const next = bytes[i++];
      const minimum = part === 1 && lead === 0xe0 ? 0xa0 : part === 1 && lead === 0xf0 ? 0x90 : 0x80;
      const maximum = part === 1 && lead === 0xed ? 0x9f : part === 1 && lead === 0xf4 ? 0x8f : 0xbf;
      requireWire(next >= minimum && next <= maximum, 'invalid-utf8');
    }
  }
}

/** Validates the complete reply before returning any views into its buffer. */
export function decodeReply(buffer: ArrayBuffer, shape: ShapeContract, expected: ExpectedReply = {},
  registry?: ReadonlyMap<number, ShapeContract>): DecodedReply {
  let queryId: bigint | undefined;
  try {
    if (buffer.byteLength >= 16) queryId = new DataView(buffer).getBigUint64(8, true);
    return decodeUnit(buffer, 0, buffer.byteLength, shape, expected, false, registry);
  } catch (error) {
    if (error instanceof WireError && error.queryId === undefined) throw new WireError(error.code,
      error.message === error.code ? '' : error.message.slice(error.code.length + 2), queryId);
    throw error;
  }
}
function decodeUnit(buffer: ArrayBuffer, base: number, byteLength: number,
  shape: ShapeContract, expected: ExpectedReply, embedded: boolean,
  registry?: ReadonlyMap<number, ShapeContract>): DecodedReply {
  requireWire(littleEndian, 'unsupported-endianness');
  requireWire(byteLength >= 128, 'truncated-header');
  const view = new DataView(buffer, base, byteLength), bytes = new Uint8Array(buffer, base, byteLength);
  requireWire(bytes[0] === 72 && bytes[1] === 52 && bytes[2] === 82 && bytes[3] === 81, 'invalid-magic');
  requireWire(view.getUint16(4, true) === WIRE_SCHEMA_VERSION, 'unknown-schema-version');
  const flags = view.getUint16(6, true);
  requireWire((flags & ~15) === 0 && view.getUint32(68, true) === 0, 'invalid-flags');
  const header: ReplyHeader = {
    flags, queryId: view.getBigUint64(8, true), campaignId: new Uint8Array(buffer, base + 16, 16),
    revision: view.getBigUint64(32, true), generation: view.getBigUint64(40, true),
    replyKind: view.getUint32(48, true), shapeId: view.getUint32(52, true),
    shapeVersion: view.getUint32(56, true), sectionCount: view.getUint32(60, true),
    unitKind: view.getUint32(64, true), unitStart: view.getBigUint64(72, true),
    unitEnd: view.getBigUint64(80, true), rowCount: view.getBigUint64(88, true),
    totalCount: view.getBigUint64(96, true), diagnosticTotal: view.getBigUint64(104, true),
    totalLength: view.getBigUint64(112, true), checksum: view.getBigUint64(120, true),
  };
  requireWire(header.totalLength === BigInt(byteLength), 'invalid-length');
  requireWire(xxh3(new Uint8Array(buffer, base + 128, byteLength - 128)) === header.checksum, 'checksum-mismatch');
  requireWire(header.replyKind >= 1 && header.replyKind <= DIAGNOSTICS_KIND, 'invalid-reply-kind');
  requireWire(header.unitKind <= 4, 'invalid-unit-kind');
  requireWire(header.shapeId === shape.id, 'unknown-shape');
  requireWire(header.shapeVersion === shape.version, 'unknown-shape-version');
  requireWire(header.replyKind === shape.replyKind && shape.unitKinds.includes(header.unitKind), 'shape-mismatch');
  requireWire(header.unitStart <= header.unitEnd && header.unitEnd - header.unitStart === header.rowCount &&
    (header.totalCount === UNKNOWN_TOTAL || header.unitEnd <= header.totalCount), 'unit-range');
  requireWire((expected.queryId === undefined || header.queryId === expected.queryId) &&
    (expected.campaignId === undefined || sameBytes(header.campaignId, expected.campaignId)) &&
    (expected.revision === undefined || header.revision === expected.revision) &&
    (expected.generation === undefined || header.generation === expected.generation), embedded ? 'diagnostics-identity' : 'reply-identity');
  requireWire(!(flags & 8) || (header.replyKind === DIAGNOSTICS_KIND && Boolean(flags & 1) && !(flags & 2)), 'invalid-diagnostics');
  requireWire(header.sectionCount <= Math.floor((byteLength - 128) / 24), 'section-bounds');
  const directoryEnd = 128 + header.sectionCount * 24;
  const sections = new Map<number, Section>();
  let priorEnd = directoryEnd;
  for (let i = 0; i < header.sectionCount; ++i) {
    const p = 128 + i * 24, kind = view.getUint32(p, true), count = view.getUint32(p + 4, true);
    requireWire(kind >= 1 && kind <= 9, 'unknown-section-kind');
    requireWire(!sections.has(kind), 'duplicate-section');
    const offset64 = view.getBigUint64(p + 8, true), length64 = view.getBigUint64(p + 16, true);
    requireWire(offset64 % 8n === 0n, 'section-alignment');
    requireWire(offset64 >= BigInt(directoryEnd) && offset64 <= BigInt(byteLength) &&
      length64 <= BigInt(byteLength) - offset64, 'section-bounds');
    if (kind === 9) requireWire(count === 1 && length64 === 24n, 'invalid-bounds');
    const offset = local(offset64, byteLength), length = local(length64, byteLength);
    requireWire(offset >= priorEnd, 'section-overlap');
    zeroPadding(bytes, priorEnd, offset); priorEnd = offset + length;
    sections.set(kind, {kind, count, offset, length});
  }
  zeroPadding(bytes, priorEnd, byteLength);
  const section = (kind: number): Section => {
    const result = sections.get(kind); requireWire(result, 'missing-section', String(kind)); return result;
  };
  const descriptorSection = section(1), topSection = section(2), bufferSection = section(3);
  requireWire(descriptorSection.length === descriptorSection.count * 64, 'descriptor-count');
  requireWire(topSection.length === topSection.count * 4, 'section-count');
  const descriptors = Array.from({length: descriptorSection.count}, (_, i) =>
    parseDescriptor(view, descriptorSection.offset + i * 64));
  const topIndices = Array.from(new Uint32Array(buffer, base + topSection.offset, topSection.count));
  const topSet = new Set(topIndices);
  requireWire(topSet.size === topIndices.length && topIndices.every(i => i < descriptors.length), 'descriptor-index');
  requireWire(descriptors.length === shape.columns.length && topIndices.length === shape.topLevel.length &&
    topIndices.every((n, i) => n === shape.topLevel[i]), 'shape-mismatch');
  const parents = Array.from({length: descriptors.length}, (): Parent | undefined => undefined);
  descriptors.forEach((d, i) => {
    requireWire(d.logicalType >= 1 && d.logicalType <= 15, 'unknown-logical-type');
    requireWire(d.encoding <= 3, 'unknown-encoding');
    requireWire((d.stateLayers & ~15) === 0, 'invalid-state-layers');
    requireWire(d.nameId < shape.names.length, 'shape-mismatch');
    requireWire((topSet.has(i) && d.length === UNKNOWN_TOTAL) || (!topSet.has(i) && d.length !== UNKNOWN_TOTAL), 'descriptor-length');
    if (d.childCount === 0) { requireWire(d.childFirst === NO_INDEX, 'descriptor-index'); return; }
    requireWire(d.childFirst !== NO_INDEX && d.childFirst < descriptors.length &&
      d.childCount <= descriptors.length - d.childFirst, 'descriptor-index');
    requireWire(d.logicalType === 14 || d.logicalType === 15, 'descriptor-parent');
    requireWire(d.logicalType !== 14 || d.childCount === 1, 'descriptor-parent');
  });
  // Iterative DFS has no chosen nesting cap and reports cycles before parent errors.
  const colours = new Uint8Array(descriptors.length);
  for (let root = 0; root < descriptors.length; ++root) {
    if (colours[root] === 2) continue;
    const stack: {column: number; next: number}[] = [{column: root, next: 0}]; colours[root] = 1;
    while (stack.length > 0) {
      const frame = stack[stack.length - 1];
      const d = descriptors[frame.column];
      if (frame.next === d.childCount) { colours[frame.column] = 2; stack.pop(); continue; }
      const child = d.childFirst + frame.next++;
      requireWire(colours[child] !== 1, 'descriptor-cycle');
      requireWire(!parents[child] && !topSet.has(child), 'descriptor-parent');
      parents[child] = {column:frame.column, list:d.logicalType === 14};
      if (colours[child] === 0) { colours[child] = 1; stack.push({column: child, next: 0}); }
    }
  }
  requireWire(parents.every((p, i) => Boolean(p) !== topSet.has(i)), 'descriptor-parent');
  // Unique ownership bounds the total child slots by the descriptor count.
  const children = descriptors.map(d => Array.from({length:d.childCount}, (_,i) => d.childFirst+i));
  const logicalLengths = descriptors.map((d, i) => topSet.has(i) ? header.rowCount : d.length);
  descriptors.forEach((d, i) => {
    if (parents[i] && !parents[i]!.list)
      requireWire(logicalLengths[i] === logicalLengths[parents[i]!.column], 'descriptor-length');
    if (d.gateColumn !== NO_GATE) {
      const parent = parents[i]; requireWire(parent && !parent.list, 'union-gate');
      const siblings = children[parent.column]; requireWire(d.gateColumn < siblings.length, 'union-gate');
      const gate = descriptors[siblings[d.gateColumn]];
      const gateName = shape.names[gate.nameId];
      requireWire(gate.logicalType === 12 && gate.dictionaryRef === NO_INDEX &&
        (gateName === '$kind' || gateName.endsWith('.$kind')) && gate.gateColumn === NO_GATE && gate.stateLayers === 0,
      'union-gate');
      const variants = shape.columns[siblings[d.gateColumn]].staticEnum;
      requireWire(variants && d.gateValue < variants.length, 'union-gate');
    } else requireWire(d.gateValue === 0, 'union-gate');
  });
  requireWire(bufferSection.count <= Math.floor(bufferSection.length / 16), 'buffer-bounds');
  const slices: Slice[] = [];
  const sortedSlices: {offset: number; length: number}[] = [];
  for (let i = 0; i < bufferSection.count; ++i) {
    const p = bufferSection.offset + i * 16;
    const offset64 = view.getBigUint64(p, true), length64 = view.getBigUint64(p + 8, true);
    requireWire(offset64 % 8n === 0n, 'buffer-alignment');
    requireWire(offset64 >= BigInt(bufferSection.count * 16) && offset64 <= BigInt(bufferSection.length) &&
      length64 <= BigInt(bufferSection.length) - offset64, 'buffer-bounds');
    const offset = local(offset64, bufferSection.length), length = local(length64, bufferSection.length);
    slices.push({offset: base + bufferSection.offset + offset, length});
    sortedSlices.push({offset, length});
  }
  sortedSlices.sort((a, b) => a.offset - b.offset || a.length - b.length);
  let bufferEnd = bufferSection.count * 16;
  for (const slice of sortedSlices) {
    requireWire(slice.offset >= bufferEnd, 'buffer-overlap');
    zeroPadding(bytes, bufferSection.offset + bufferEnd, bufferSection.offset + slice.offset);
    bufferEnd = slice.offset + slice.length;
  }
  zeroPadding(bytes, bufferSection.offset + bufferEnd, bufferSection.offset + bufferSection.length);
  const slice = (index: number): Slice => { requireWire(index < slices.length, 'buffer-index'); return slices[index]; };
  const dictionaries: DictionaryReference[] = [];
  const dictionarySection = sections.get(4);
  if (dictionarySection) {
    requireWire(dictionarySection.length === dictionarySection.count * 88, 'section-count');
    for (let i = 0; i < dictionarySection.count; ++i) {
      const p = dictionarySection.offset + i * 88;
      const reference = {identity: new Uint8Array(buffer, base + p, 32),
        shardIdentity: new Uint8Array(buffer, base + p + 32, 32),
        firstEntry: view.getBigUint64(p + 64, true), entryCount: view.getBigUint64(p + 72, true),
        kind: view.getUint32(p + 80, true), flags: view.getUint32(p + 84, true)};
      requireWire(reference.kind <= 5, 'dictionary-kind');
      requireWire(reference.flags === 0, 'dictionary-reference');
      // An empty shard may start at MAX; the last actual entry must fit u64.
      if (reference.entryCount > 0n) checkedAdd(reference.firstEntry, reference.entryCount - 1n, 'dictionary-overflow');
      dictionaries.push(reference);
    }
  }
  const columns: ColumnView[] = [];
  descriptors.forEach((d, index) => {
    const count = logicalLengths[index];
    const stateViews: (ScalarView | undefined)[] = Array.from({length: 4});
    let stateOffset = 0;
    for (let layer = 0; layer < 4; ++layer) {
      const width = (d.stateWidths >>> (layer * 4)) & 15;
      if (!(d.stateLayers & (1 << layer))) { requireWire(width === 0, 'invalid-state-width'); continue; }
      requireWire(width <= 4, 'invalid-state-width');
      const stateSlice = slice(d.stateBuffer + stateOffset++);
      stateViews[layer] = width === 0 ? packedScalar(buffer, stateSlice, count, 1, false) :
        plainScalar(buffer, stateSlice, count, 8 * 2 ** (width - 1), false);
    }
    requireWire(stateOffset > 0 || d.stateBuffer === NO_INDEX, 'invalid-state-layers');
    if (d.stateLayers & 8) {
      requireWire(d.refusalRef < dictionaries.length && dictionaries[d.refusalRef].kind === 3, 'refusal-reference');
    } else requireWire(d.refusalRef === NO_INDEX, 'refusal-reference');
    if (d.dictionaryRef !== NO_INDEX) {
      requireWire((d.logicalType === 12 || d.logicalType === 13) &&
        !shape.columns[index].staticEnum && d.dictionaryRef < dictionaries.length, 'dictionary-reference');
      if (shape.columns[index].dictionaryKind !== undefined)
        requireWire(dictionaries[d.dictionaryRef].kind === shape.columns[index].dictionaryKind, 'dictionary-kind');
      if (d.logicalType === 13) requireWire(dictionaries[d.dictionaryRef].kind === 3, 'dictionary-kind');
    } else if (d.logicalType === 12 || d.logicalType === 13) {
      // A fully inactive parent may contain an unbound dynamic code; it is checked only on meaningful entries.
    }
    let scalar: ScalarView | undefined, offsets: BigUint64Array | undefined,
      byteData: Uint8Array | undefined, lo: BigUint64Array | undefined, hi: BigUint64Array | undefined;
    let wideIndex = (i: number | bigint) => local(elementIndex(i, count), Number.MAX_SAFE_INTEGER);
    if (d.logicalType === 14 || d.logicalType === 10 || d.logicalType === 11) {
      requireWire(d.encoding === 0 && d.paramB === 0 &&
        (d.logicalType === 14 ? d.paramA === 64 && d.childCount === 1 : d.paramA === 0 && d.childCount === 0), 'invalid-type-parameter');
      const auxiliary = slice(d.auxBuffer);
      requireWire((count + 1n) * 8n === BigInt(auxiliary.length), 'offsets-length');
      offsets = new BigUint64Array(buffer, auxiliary.offset, local(count + 1n, auxiliary.length / 8));
      const bound = d.logicalType === 14 ? logicalLengths[children[index][0]] : BigInt(slice(d.dataBuffer).length);
      if (bound <= BigInt(Number.MAX_SAFE_INTEGER)) {
        // These local bounds admit exact number coordinates. Read limbs without allocating BigInts.
        const offsetView = new DataView(buffer, auxiliary.offset, auxiliary.length), maximum = Number(bound);
        let previous = 0;
        for (let i = 0; i < offsets.length; i++) {
          const value = offsetView.getUint32(i*8,true) + offsetView.getUint32(i*8+4,true)*0x100000000;
          requireWire((i !== 0 || value === 0) && value >= previous, 'offsets-order');
          requireWire(value <= maximum, 'offsets-bounds');
          previous = value;
        }
        requireWire(previous === maximum, 'offsets-bounds');
      } else {
        let previous = 0n;
        for (let i = 0; i < offsets.length; i++) {
          const value = offsets[i];
          requireWire((i !== 0 || value === 0n) && value >= previous, 'offsets-order');
          previous = value;
        }
        requireWire(previous === bound, 'offsets-bounds');
      }
      if (d.logicalType !== 14) { const data = slice(d.dataBuffer); byteData = new Uint8Array(buffer, data.offset, data.length); }
      else requireWire(d.dataBuffer === NO_INDEX, 'buffer-index');
    } else if (d.logicalType === 15) {
      requireWire(d.encoding === 0 && d.paramA === 0 && d.paramB === 0 &&
        d.dataBuffer === NO_INDEX && d.auxBuffer === NO_INDEX, 'invalid-type-parameter');
    } else {
      requireWire(d.childCount === 0, 'descriptor-parent');
      let width: number, signed = false, floating = false, wide = false;
      switch (d.logicalType) {
        case 1: width = 1; requireWire(d.paramA === 0 && (d.encoding !== 1 || d.paramB === 1), 'invalid-type-parameter'); break;
        case 2: case 3: case 12: case 13: width = d.paramA; integerWidth(width); signed = d.logicalType === 3; break;
        case 4: width = 64; floating = true; requireWire(d.paramA === 0 || d.paramA === 64, 'invalid-type-parameter'); break;
        case 5: width = 128; wide = true; signed = d.paramA === 1; requireWire(d.paramA <= 1, 'invalid-type-parameter'); break;
        case 6: width = d.paramA & 255; signed = (d.paramA & 256) !== 0; wide = width === 128;
          requireWire((d.paramA & ~511) === 0 && [8,16,32,64,128].includes(width) &&
            (d.paramB & 255) >= 2 && ((d.paramB & 255) === 2 || (d.paramB & 255) === 10) &&
            d.paramB <= 65535, 'invalid-type-parameter'); break;
        case 7: width = d.paramA; floating = true; requireWire(width === 32 || width === 64, 'invalid-type-parameter'); break;
        default: width = 64; signed = true; requireWire(d.paramA === 0 || d.paramA === 64, 'invalid-type-parameter');
      }
      requireWire(d.encoding !== 2 || d.logicalType === 12 || d.logicalType === 13, 'invalid-type-parameter');
      requireWire(!floating || d.encoding === 0 || d.encoding === 3, 'invalid-type-parameter');
      const data = slice(d.dataBuffer);
      if (wide) {
        requireWire(d.encoding === 0 || d.encoding === 3, 'invalid-type-parameter');
        let physicalCount = count;
        if (d.encoding === 3) {
          const auxiliary = slice(d.auxBuffer); requireWire(auxiliary.length % 4 === 0, 'run-length');
          const runs = new Uint32Array(buffer, auxiliary.offset, auxiliary.length / 4);
          const positions = runScalar({length: runs.length, get(i) { return typeof i === 'bigint' ? i : i; }}, runs, count);
          physicalCount = BigInt(runs.length); wideIndex = i => Number(positions.get(i));
        }
        requireWire(BigInt(data.length) === physicalCount * 16n, 'buffer-length');
        lo = new BigUint64Array(buffer, data.offset, local(physicalCount, data.length / 16));
        hi = new BigUint64Array(buffer, data.offset + lo.length * 8, lo.length);
      } else if (d.encoding === 1) {
        requireWire(d.logicalType !== 6 && d.logicalType !== 8 && d.logicalType !== 9 &&
          !floating && d.paramB <= width, 'invalid-type-parameter');
        scalar = packedScalar(buffer, data, count, d.paramB, signed);
      } else if (d.encoding === 3) {
        const auxiliary = slice(d.auxBuffer); requireWire(auxiliary.length % 4 === 0, 'run-length');
        const runs = new Uint32Array(buffer, auxiliary.offset, auxiliary.length / 4);
        const values = width === 1 ? packedScalar(buffer, data, BigInt(runs.length), 1, false) :
          plainScalar(buffer, data, BigInt(runs.length), width, signed, floating);
        scalar = runScalar(values, runs, count);
      } else scalar = width === 1 ? packedScalar(buffer, data, count, 1, false) :
        plainScalar(buffer, data, count, width, signed, floating);
      requireWire(d.encoding === 3 || d.auxBuffer === NO_INDEX, 'buffer-index');
    }
    columns.push({descriptor: d, get length() { return local(count, Number.MAX_SAFE_INTEGER); }, logicalLength: count, scalar, data: scalar?.data,
      lo, hi, offsets, bytes: byteData, children: children[index], states: stateViews,
      state(i) { elementIndex(i, count); return valueState(index, i); },
      value(i) {
        if (!this.state(i).present) return undefined;
        if (scalar) return scalar.get(i);
        if (lo && hi) { const physical = wideIndex(i), raw = (hi[physical] << 64n) | lo[physical];
          const signed = d.logicalType === 5 ? d.paramA === 1 : (d.paramA & 256) !== 0;
          return signed ? BigInt.asIntN(128, raw) : raw; }
        if (offsets) {
          const coordinate = local(elementIndex(i, count), offsets.length - 2, 'descriptor-index');
          if (byteData) return byteData.subarray(local(offsets[coordinate], byteData.length), local(offsets[coordinate + 1], byteData.length));
          return {first: offsets[coordinate], end: offsets[coordinate + 1]};
        }
        return undefined;
      }});
  });
  function ownState(column: number, index: number | bigint): ValueState {
    const result: (bigint | null)[] = [null, null, null, null];
    for (let layer = 0; layer < 4; ++layer) {
      const state = columns[column].states[layer]; if (!state) continue;
      result[layer] = BigInt(state.get(index));
      if (result[layer] !== 0n) return {active: true, present: false, layers: result};
    }
    return {active: true, present: true, layers: result};
  }
  function valueState(column: number, index: number | bigint): ValueState {
    const ancestry: {column: number; index: number | bigint}[] = [];
    let currentColumn = column, currentIndex: number | bigint = index;
    while (true) {
      ancestry.push({column: currentColumn, index: currentIndex});
      const parent = parents[currentColumn]; if (!parent) break;
      if (parent.list) currentIndex = listParent(columns[parent.column].offsets!, currentIndex);
      currentColumn = parent.column;
    }
    for (let i = ancestry.length - 1; i >= 0; --i) {
      const occurrence = ancestry[i], d = descriptors[occurrence.column];
      if (d.gateColumn !== NO_GATE) {
        const parent = parents[occurrence.column]!;
        const gate = children[parent.column][d.gateColumn];
        if (BigInt(columns[gate].scalar!.get(occurrence.index)) !== BigInt(d.gateValue))
          return {active: false, present: false, layers: [null,null,null,null]};
      }
      const state = ownState(occurrence.column, occurrence.index);
      if (!state.present) return state;
    }
    return ownState(column, index);
  }
  descriptors.forEach((d, index) => {
    const expectedDescriptor = shape.columns[index].descriptor;
    for (const key of SHAPE_DESCRIPTOR_KEYS)
      if (d[key] !== (expectedDescriptor[key] ?? SHAPE_DESCRIPTOR_DEFAULTS[key as keyof typeof SHAPE_DESCRIPTOR_DEFAULTS]))
        throw new WireError('shape-mismatch', `${shape.names[d.nameId]}.${key}`);
  });
  const extents: Extent[] = [], extentMap = new Map<string, Extent>();
  const extentSection = sections.get(8);
  if (extentSection) {
    requireWire(extentSection.length === extentSection.count * 40, 'invalid-extent');
    for (let i = 0; i < extentSection.count; ++i) {
      const p = extentSection.offset + i * 40;
      const column = view.getUint32(p, true), coordinate = view.getUint8(p + 4), final = view.getUint8(p + 5);
      requireWire(column < columns.length && final <= 1 && view.getUint16(p + 6, true) === 0, 'invalid-extent');
      const d = descriptors[column], index64 = view.getBigUint64(p + 8, true);
      requireWire((d.logicalType === 14 && coordinate === 1) || ((d.logicalType === 10 || d.logicalType === 11) && coordinate === 2), 'invalid-extent');
      requireWire(index64 < logicalLengths[column], 'invalid-extent');
      const index = local(index64, columns[column].length - 1, 'invalid-extent');
      const first = view.getBigUint64(p + 16, true), end = view.getBigUint64(p + 24, true), total = view.getBigUint64(p + 32, true);
      const offsets = columns[column].offsets!;
      requireWire(first <= end && end <= total && end - first === offsets[index + 1] - offsets[index] &&
        (total === UNKNOWN_TOTAL || Boolean(final) === (end === total)), 'invalid-extent');
      const key = `${column}:${index64}`; requireWire(!extentMap.has(key), 'duplicate-extent');
      const extent = {column, coordinate, final: final === 1, index: index64, first, end, total};
      extents.push(extent); extentMap.set(key, extent);
    }
  }
  // Undefined means uniformly present; other activity ranges are coalesced, half-open.
  type Activity = readonly bigint[] | undefined;
  const activity: Activity[] = new Array(columns.length);
  const valueExtents = new Map<number, Map<number, Extent>>();
  for (const extent of extents) {
    let entries = valueExtents.get(extent.column);
    if (!entries) { entries = new Map(); valueExtents.set(extent.column, entries); }
    entries.set(Number(extent.index), extent);
  }
  function appendRange(out: bigint[], first: bigint, end: bigint): void {
    if (first === end) return;
    if (out.length && out[out.length - 1] === first) out[out.length - 1] = end;
    else out.push(first, end);
  }
  function normalize(ranges: bigint[], count: bigint): Activity {
    return ranges.length === 2 && ranges[0] === 0n && ranges[1] === count ? undefined : ranges;
  }
  function intersect(left: Activity, right: Activity): Activity {
    if (!left) return right;
    if (!right) return left;
    const out: bigint[] = [];
    let a = 0, b = 0;
    while (a < left.length && b < right.length) {
      const first = left[a] > right[b] ? left[a] : right[b];
      const end = left[a+1] < right[b+1] ? left[a+1] : right[b+1];
      if (first < end) appendRange(out, first, end);
      if (left[a+1] <= right[b+1]) a += 2; else b += 2;
    }
    return out;
  }
  const work = [...topIndices];
  for (let cursor = 0; cursor < work.length; cursor++) {
    const index = work[cursor], column = columns[index], d = descriptors[index], spec = shape.columns[index];
    const count = column.logicalLength, parent = parents[index];
    let ranges = parent ? activity[parent.column] : undefined;
    if (parent?.list && ranges) {
      const offsets = columns[parent.column].offsets!, mapped: bigint[] = [];
      for (let r = 0; r < ranges.length; r += 2)
        appendRange(mapped, offsets[Number(ranges[r])], offsets[Number(ranges[r+1])]);
      ranges = normalize(mapped, count);
    }
    if (d.gateColumn !== NO_GATE && (!ranges || ranges.length)) {
      const gate = columns[children[parent!.column][d.gateColumn]].scalar!;
      const matched: bigint[] = [];
      if (gate.runEnds) {
        let first = 0n;
        for (let physical = 0; physical < gate.runEnds.length; physical++) {
          const end = gate.runEnds[physical], value = gate.runValues!.get(physical);
          if (value === d.gateValue || value === BigInt(d.gateValue)) appendRange(matched, first, end);
          first = end;
        }
      } else {
        const n = gate.length;
        for (let r = 0; r < (ranges?.length ?? 2); r += 2) {
          const first = ranges ? Number(ranges[r]) : 0, end = ranges ? Number(ranges[r+1]) : n;
          let start = -1;
          for (let i = first; i < end; i++) {
            const value = gate.get(i);
            const present = value === d.gateValue || value === BigInt(d.gateValue);
            if (present && start < 0) start = i;
            if (!present && start >= 0) { appendRange(matched, BigInt(start), BigInt(i)); start = -1; }
          }
          if (start >= 0) appendRange(matched, BigInt(start), BigInt(end));
        }
      }
      ranges = intersect(ranges, normalize(matched, count));
    }
    if (d.stateLayers && (!ranges || ranges.length) &&
      !column.states.every(state => !state || state.data!.every(code => code === 0 || code === 0n))) {
      // State buffers have received-byte-bounded local counts; no ancestry or occurrence allocation.
      const present: bigint[] = [];
      const n = column.states.find(s => s !== undefined)!.length;
      for (let r = 0; r < (ranges?.length ?? 2); r += 2) {
        const first = ranges ? Number(ranges[r]) : 0, end = ranges ? Number(ranges[r+1]) : n;
        let start = -1;
        for (let i = first; i < end; i++) {
          let meaningful = true;
          for (let layer = 0; layer < 4; layer++) {
            const state = column.states[layer]; if (!state) continue;
            const code = state.get(i);
            if (layer <= 1) requireWire(code <= 2, 'invalid-state-code');
            else if (layer === 2) requireWire(code <= 1, 'invalid-state-code');
            else if (code > 0) requireWire(BigInt(code) - 1n < dictionaries[d.refusalRef].entryCount, 'dictionary-code');
            if (code !== 0 && code !== 0n) { meaningful = false; break; }
          }
          if (meaningful && start < 0) start = i;
          if (!meaningful && start >= 0) { appendRange(present, BigInt(start), BigInt(i)); start = -1; }
        }
        if (start >= 0) appendRange(present, BigInt(start), BigInt(end));
      }
      ranges = normalize(present, count);
    }
    activity[index] = ranges;
    for (const child of children[index]) work.push(child);
    if (ranges && !ranges.length) continue;
    const scalarChecks = d.logicalType === 4 || d.logicalType === 12 || d.logicalType === 13;
    if (!scalarChecks && d.logicalType !== 11 && spec.fixedListLength === undefined) continue;
    const limit = d.dictionaryRef === NO_INDEX ? spec.staticEnum?.length : dictionaries[d.dictionaryRef].entryCount;
    const numberLimit = limit === undefined ? undefined : Number(limit);
    function validateScalar(value: Scalar): void {
      if (d.logicalType === 12 || d.logicalType === 13) {
        requireWire(limit !== undefined, 'dictionary-reference');
        requireWire(typeof value === 'number' ? value >= 0 && (Number.isSafeInteger(numberLimit)
          ? value < numberLimit! : BigInt(value) < BigInt(limit)) : value >= 0n && value < BigInt(limit), 'dictionary-code');
      } else requireWire(typeof value === 'number' && Number.isInteger(value) && Math.abs(value) <= 2 ** 53, 'invalid-safe53');
    }
    if (d.encoding === 3 && scalarChecks) {
      const scalar = column.scalar!, ends = scalar.runEnds!, values = scalar.runValues!;
      let first = 0n, range = 0;
      for (let physical = 0; physical < ends.length; physical++) {
        const end = ends[physical];
        while (ranges && range < ranges.length && ranges[range+1] <= first) range += 2;
        if (!ranges || (range < ranges.length && ranges[range] < end)) validateScalar(values.get(physical));
        first = end;
      }
      continue;
    }
    const n = column.length;
    for (let r = 0; r < (ranges?.length ?? 2); r += 2) {
      const first = ranges ? Number(ranges[r]) : 0, end = ranges ? Number(ranges[r+1]) : n;
      for (let i = first; i < end; i++) {
        if (scalarChecks) validateScalar(column.scalar!.get(i));
        if (d.logicalType === 11) validateText(column.bytes!, Number(column.offsets![i]),
          Number(column.offsets![i+1]), valueExtents.get(index)?.get(i));
        if (spec.fixedListLength !== undefined && column.offsets)
          requireWire((valueExtents.get(index)?.get(i)?.total ?? column.offsets[i+1] - column.offsets[i]) ===
            spec.fixedListLength, 'descriptor-length');
      }
    }
  }
  const continuationSection = sections.get(6), diagnosticContinuationSection = sections.get(7);
  requireWire(Boolean(flags & 2) === Boolean(continuationSection), 'continuation-flags');
  for (const token of [continuationSection, diagnosticContinuationSection])
    if (token) requireWire(token.count === 1, 'section-count');
  let bounds: Bounds | undefined;
  const boundsSection = sections.get(9);
  const diagnosticsReply = header.replyKind === DIAGNOSTICS_KIND;
  requireWire(!diagnosticsReply || !boundsSection, 'invalid-bounds');
  requireWire(((shape.bounds !== 'none' && shape.bounds !== 'forbidden') || !boundsSection) &&
    (shape.bounds !== 'required' || Boolean(boundsSection)), 'invalid-bounds');
  requireWire(Boolean(boundsSection) === Boolean(expected.bounds), 'bounds-mismatch');
  if (boundsSection) {
    requireWire(boundsSection.count === 1 && boundsSection.length === 24, 'invalid-bounds');
    const p = boundsSection.offset;
    bounds = {kind: view.getUint32(p, true), start: view.getBigInt64(p + 8, true), end: view.getBigInt64(p + 16, true)};
    requireWire(bounds.kind <= 2 && view.getUint32(p + 4, true) === 0 && bounds.start <= bounds.end, 'invalid-bounds');
    requireWire(bounds.kind === expected.bounds!.kind && bounds.start === expected.bounds!.start && bounds.end === expected.bounds!.end, 'bounds-mismatch');
  }
  const diagnosticsSection = sections.get(5);
  requireWire(!(embedded || diagnosticsReply) || !diagnosticsSection, 'invalid-diagnostics');
  let diagnostics: DecodedReply | undefined;
  if (diagnosticsSection) {
    requireWire(diagnosticsSection.count === 1, 'invalid-diagnostics');
    requireWire(diagnosticsSection.length >= 128 &&
      view.getUint32(diagnosticsSection.offset + 48, true) === DIAGNOSTICS_KIND, 'invalid-diagnostics');
    const diagnosticShapeId = view.getUint32(diagnosticsSection.offset + 52, true);
    const diagnosticShape = expected.diagnostics ?? shape.diagnostics ?? registry?.get(diagnosticShapeId);
    requireWire(diagnosticShape && diagnosticShape.replyKind === DIAGNOSTICS_KIND, 'invalid-diagnostics');
    diagnostics = decodeUnit(buffer, base + diagnosticsSection.offset, diagnosticsSection.length, diagnosticShape,
      {queryId: header.queryId, campaignId: header.campaignId, revision: header.revision, generation: header.generation}, true, registry);
    requireWire(diagnostics.header.totalCount === header.diagnosticTotal, 'invalid-diagnostics');
  }
  const asBytes = (s?: Section) => s ? new Uint8Array(buffer, base + s.offset, s.length) : undefined;
  return {buffer, header, shape, columns, topLevel: topIndices.map(i => columns[i]), dictionaries, extents, bounds,
    continuation: asBytes(continuationSection), diagnosticContinuation: asBytes(diagnosticContinuationSection), diagnostics,
    originalIdentity(column, index) {
      requireWire(Number.isSafeInteger(column) && column >= 0 && column < columns.length, 'descriptor-index');
      elementIndex(index, columns[column].logicalLength);
      const elements: bigint[] = [];
      let current = column, coordinate: number | bigint = index;
      while (parents[current]) {
        const parent = parents[current]!;
        if (parent.list) {
          const list = columns[parent.column], owner = listParent(list.offsets!, coordinate);
          const extent = extentMap.get(`${parent.column}:${owner}`);
          elements.push(checkedAdd(extent?.first ?? 0n, BigInt(coordinate) - list.offsets![owner]));
          coordinate = owner;
        }
        current = parent.column;
      }
      return {row: checkedAdd(header.unitStart, BigInt(coordinate)), elements: elements.reverse()};
    }};
}

/** Decode text only when its display is needed; byte columns retain arbitrary bytes. */
export function displayText(bytes: Uint8Array): string { return utf8.decode(bytes); }
