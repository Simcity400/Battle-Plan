import { performance } from 'node:perf_hooks';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { decodeReply } from './runtime.ts';
import type { ExpectedReply, ShapeContract } from './runtime.ts';
import { bytesOf, encodeSynthetic, syntheticShape, u64 } from './test-support.ts';
import type { SyntheticReply } from './test-support.ts';
import { loadFixtureManifest } from './fixture-manifest.ts';

/** These are benchmark request sizes, not admission or campaign limits. */
const request = {
  runs: Number(process.env.H4R_BENCH_WINDOW_RUNS ?? 8760),
  countries: Number(process.env.H4R_BENCH_VIEWPORT_COUNTRIES ?? 5000),
  completed: Number(process.env.H4R_BENCH_COMPLETIONS_PER_COUNTRY ?? 20),
  iterations: Number(process.env.H4R_BENCH_ITERATIONS ?? 80),
};
for (const [name, value] of Object.entries(request)) {
  if (!Number.isSafeInteger(value) || value < 1) throw new Error(`invalid benchmark configuration: ${name}`);
}
interface Case {
  name: string; buffer: ArrayBuffer; shape: ShapeContract; expected: ExpectedReply;
  registry?: ReadonlyMap<number,ShapeContract>;
}
function makeCase(name: string, reply: SyntheticReply, names: string[]): Case {
  return {name, buffer: encodeSynthetic(reply), shape: syntheticShape(reply, names),
    expected: {bounds: reply.bounds}};
}

function syntheticCases(): Case[] {
  const n = request.runs;
  const first = Int32Array.from({length:n}, (_, i) => 876_000 + i);
  const bits = new Uint8Array(Math.ceil(n / 8)); bits[0] = 1;
  const chart: SyntheticReply = {
    shapeId: 90001, replyKind: 1, unitKind: 1, rowCount: 1n,
    bounds: {kind:0,start:876000n,end:876000n+BigInt(n)},
    descriptors: [
      {nameId:0,logicalType:15,childFirst:6,childCount:1},
      {nameId:1,logicalType:2,paramA:64,dataBuffer:0},
      {nameId:2,logicalType:14,paramA:64,childFirst:7,childCount:1,auxBuffer:1},
      {nameId:3,logicalType:14,paramA:64,childFirst:8,childCount:1,auxBuffer:2},
      {nameId:4,logicalType:10,auxBuffer:3,dataBuffer:4},
      {nameId:5,logicalType:10,auxBuffer:5,dataBuffer:6},
      {nameId:6,logicalType:2,paramA:64,dataBuffer:7,length:1n},
      {nameId:7,logicalType:15,childFirst:9,childCount:4,length:BigInt(n)},
      {nameId:8,logicalType:2,paramA:32,dataBuffer:8,length:1n},
      {nameId:9,logicalType:3,paramA:32,dataBuffer:9,length:BigInt(n)},
      {nameId:10,logicalType:3,paramA:32,dataBuffer:10,length:BigInt(n)},
      {nameId:11,logicalType:7,paramA:64,dataBuffer:11,length:BigInt(n)},
      {nameId:12,logicalType:1,encoding:1,paramB:1,dataBuffer:12,length:BigInt(n)},
    ], topLevel:[0,1,2,3,4,5],
    buffers:[u64([4_380_000_000n]),u64([0n,BigInt(n)]),u64([0n,1n]),
      u64([0n,BigInt(n*10)]),new Uint8Array(n*10),u64([0n,64n]),new Uint8Array(64),
      u64([1n]),bytesOf(new Uint32Array([0])),bytesOf(first),bytesOf(first),
      bytesOf(Float64Array.from({length:n}, (_, i) => Math.sin(i))),bits],
  };
  const countries = request.countries, entries = countries * request.completed;
  const focus: SyntheticReply = {
    shapeId:90002,replyKind:1,unitKind:2,rowCount:BigInt(countries),
    bounds:{kind:1,start:0n,end:BigInt(countries)},
    descriptors:[
      {nameId:0,logicalType:2,paramA:64,dataBuffer:0},
      {nameId:1,logicalType:14,paramA:64,childFirst:2,childCount:1,auxBuffer:1},
      {nameId:2,logicalType:15,childFirst:3,childCount:3,length:BigInt(entries)},
      {nameId:3,logicalType:2,paramA:64,dataBuffer:2,length:BigInt(entries)},
      {nameId:4,logicalType:8,paramA:64,dataBuffer:3,length:BigInt(entries)},
      {nameId:5,logicalType:6,paramA:320,paramB:1290,dataBuffer:4,length:BigInt(entries)},
    ],topLevel:[0,1],
    buffers:[bytesOf(BigUint64Array.from({length:countries},(_,i)=>BigInt(i))),
      bytesOf(BigUint64Array.from({length:countries+1},(_,i)=>BigInt(i*request.completed))),
      bytesOf(BigUint64Array.from({length:entries},(_,i)=>4_380_000_000n+BigInt(i))),
      bytesOf(BigInt64Array.from({length:entries},(_,i)=>876_000n+BigInt(i))),
      bytesOf(BigInt64Array.from({length:entries},(_,i)=>BigInt(i)*100_000n))],
  };
  const produced: SyntheticReply = {
    shapeId:90003,replyKind:6,unitKind:3,rowCount:BigInt(countries),
    bounds:{kind:2,start:0n,end:BigInt(countries)},
    descriptors:[{nameId:0,logicalType:2,paramA:64,dataBuffer:0},
      {nameId:1,logicalType:5,paramA:1,dataBuffer:1},
      {nameId:2,logicalType:2,paramA:64,dataBuffer:2}],topLevel:[0,1,2],
    buffers:[bytesOf(BigUint64Array.from({length:countries},(_,i)=>BigInt(i))),
      bytesOf(BigUint64Array.from({length:countries*2},(_,i)=>i<countries?(1n<<60n)+BigInt(i):0n)),
      bytesOf(BigUint64Array.from({length:countries},(_,i)=>4_380_000_000n+BigInt(i)))],
  };
  return [makeCase('chart-window',chart,['series','first_run','runs','segment_starts','tree','directory',
    'series.key','runs.$item','segment_starts.$item','runs.$item.first','runs.$item.last','runs.$item.value','runs.$item.break_before']),
    makeCase('focus-viewport-layout',focus,['country','completed','completed.$item','completed.$item.ordinal',
      'completed.$item.completed_by_hour','completed.$item.progress_raw']),
    makeCase('produced-select-layout',produced,['country','score','rank'])];
}
const cases = process.env.H4R_WIRE_FIXTURES ? (() => {
  const folder = resolve(process.env.H4R_WIRE_FIXTURES!);
  const manifest = loadFixtureManifest(folder);
  return manifest.fixtures.filter(f => !f.expectedError && f.benchmark).map(f => {
    const bytes = readFileSync(resolve(folder, f.file));
    return {name:f.name,buffer:bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength),
      shape:manifest.shapes.get(f.shapeId)!,expected:{bounds:f.bounds},registry:manifest.shapes};
  });
})() : syntheticCases();
if (cases.length === 0) throw new Error('No benchmark fixtures: export wire-fixtures first.');

const output = cases.map(item => {
  for (let i=0;i<10;i++) decodeReply(item.buffer,item.shape,item.expected,item.registry);
  const costs: number[] = [];
  for (let i=0;i<request.iterations;i++) {
    const start = performance.now();
    decodeReply(item.buffer,item.shape,item.expected,item.registry);
    costs.push(performance.now()-start);
  }
  costs.sort((a,b)=>a-b);
  return {name:item.name,bytes:item.buffer.byteLength,iterations:request.iterations,
    medianMs:costs[Math.floor(costs.length/2)],p99Ms:costs[Math.min(costs.length-1,Math.ceil(costs.length*.99)-1)]};
});
console.log(JSON.stringify({node:process.version,request,fixtureSource:process.env.H4R_WIRE_FIXTURES ? 'Rust' : 'synthetic TS layouts',decode:output},null,2));
