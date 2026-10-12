import { performance } from 'node:perf_hooks';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { loadFixtureManifest } from './fixture-manifest.ts';
import { bytesOf, encodeSynthetic, u64 } from './test-support.ts';
import type { SyntheticReply } from './test-support.ts';

const { decodeReply } = await import(process.env.H4R_BENCH_RUNTIME
  ? pathToFileURL(resolve(process.env.H4R_BENCH_RUNTIME)).href : './runtime.ts') as typeof import('./runtime.ts');

// Rebuild smaller scheduling units from the actual registered Rust focus fixture.
const folder = process.env.H4R_WIRE_FIXTURES;
if (!folder) throw new Error('Set H4R_WIRE_FIXTURES to the Rust fixture export.');
const manifest = loadFixtureManifest(folder);
const fixture = manifest.fixtures.find(f => f.name === 'window-focus-16384-segments')!;
const source = readFileSync(resolve(folder, fixture.file));
const buffer = source.buffer.slice(source.byteOffset, source.byteOffset + source.byteLength);
const shape = manifest.shapes.get(fixture.shapeId)!;
const original = decodeReply(buffer, shape, { bounds: fixture.bounds }, manifest.shapes);
const table = new DataView(buffer);
let dictionaries: Uint8Array | undefined;
for (let i = 0; i < original.header.sectionCount; i++) {
  const p = 128 + i * 24;
  if (table.getUint32(p, true) === 4) dictionaries = new Uint8Array(buffer,
    Number(table.getBigUint64(p + 8, true)), Number(table.getBigUint64(p + 16, true)));
}
const iterations = Number(process.env.H4R_BENCH_ITERATIONS ?? 300);
const warmups = Number(process.env.H4R_BENCH_WARMUPS ?? 200);
for (const value of [iterations,warmups])
  if (!Number.isSafeInteger(value) || value < 1) throw new Error('Invalid focus benchmark sample count.');
const output = [256, 512, 1024, 16384].map(n => {
  const buffers: Uint8Array[] = [];
  const reply: SyntheticReply = {
    shapeId: shape.id, shapeVersion: shape.version, replyKind: shape.replyKind, unitKind: original.header.unitKind,
    rowCount: 1n, topLevel: [...shape.topLevel], descriptors: [], buffers,
    bounds: { kind: 0, start: 100n, end: 100n + BigInt(n) },
    extraSections: dictionaries ? [{ kind: 4, count: original.dictionaries.length, bytes: dictionaries }] : [],
  };
  const add = (bytes: Uint8Array) => { buffers.push(bytes); return buffers.length - 1; };
  for (const column of original.columns) {
    const d = { ...column.descriptor };
    const count = column.logicalLength === 16384n ? n : Number(column.logicalLength);
    if (d.length === 16384n) d.length = BigInt(n);
    if (column.offsets) {
      d.auxBuffer = add(column.offsets.length === 2 && column.offsets[1] === 16384n
        ? u64([0n, BigInt(n)]) : bytesOf(column.offsets.subarray(0, count + 1)));
    }
    if (column.data) d.dataBuffer = add(bytesOf(column.data.subarray(0,
      d.encoding === 1 ? Math.ceil(count * d.paramB / 8) : count)));
    if (column.bytes) d.dataBuffer = add(column.bytes);
    if (d.stateLayers) {
      d.stateBuffer = buffers.length;
      for (let layer = 0; layer < 4; layer++) {
        const state = column.states[layer]; if (!state) continue;
        const width = (d.stateWidths >>> (layer * 4)) & 15;
        add(bytesOf(state.data!.subarray(0, width === 0 ? Math.ceil(count / 8) : count)));
      }
    }
    reply.descriptors.push(d);
  }
  const unit = encodeSynthetic(reply);
  for (let i = 0; i < warmups; i++) decodeReply(unit, shape, { bounds: reply.bounds }, manifest.shapes);
  const costs: number[] = [];
  for (let i = 0; i < iterations; i++) {
    const start = performance.now();
    decodeReply(unit, shape, { bounds: reply.bounds }, manifest.shapes);
    costs.push(performance.now() - start);
  }
  costs.sort((a, b) => a - b);
  return { segments: n, bytes: unit.byteLength, medianMs: costs[Math.floor(iterations / 2)],
    p99Ms: costs[Math.ceil(iterations * .99) - 1] };
});
console.log(JSON.stringify({ node: process.version, iterations, warmups, source: 'registered Rust focus contract and physical fixture',
  includes: 'checksum and complete decode; construction outside timed region', decode: output }, null, 2));
