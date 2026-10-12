import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import test from 'node:test';
import { decodeReply, displayText, NO_INDEX, WireError } from './runtime.ts';
import type { ColumnView, DecodedReply, ColumnSpec } from './runtime.ts';
import { loadFixtureManifest } from './fixture-manifest.ts';
import { decodeDictionaryShard, validateDictionaryExtent } from './dictionary.ts';
import { encodeSynthetic, syntheticShape, u64 } from './test-support.ts';
import type { SyntheticReply } from './test-support.ts';

const folder = process.env.H4R_WIRE_FIXTURES;

function samePhysicalBytes(view: ArrayBufferView | undefined, hex: string): void {
  assert.ok(view);
  assert.deepEqual(Buffer.from(view.buffer,view.byteOffset,view.byteLength),Buffer.from(hex,'hex'));
}
function checkConformanceText(column: ColumnView): void {
  assert.deepEqual(Array.from(column.offsets!),[0n,2n,5n,14n]);
  samePhysicalBytes(column.bytes,'c3a9e2888573796e746865746963');
  assert.deepEqual([0,1,2].map(i=>displayText(column.value(i) as Uint8Array)),['\u00e9','\u2205','synthetic']);
}
test('conformance text offsets cover complete multibyte values and refuse the corrupted payload', () => {
  const reply: SyntheticReply = {shapeId:9000,replyKind:4,rowCount:3n,
    descriptors:[{nameId:0,logicalType:11,auxBuffer:0,dataBuffer:1}],topLevel:[0],
    buffers:[u64([0n,2n,5n,14n]),Buffer.from('c3a9e2888573796e746865746963','hex')]};
  const shape = syntheticShape(reply,['text']);
  checkConformanceText(decodeReply(encodeSynthetic(reply),shape).columns[0]);
  reply.buffers[1]=Buffer.from('c383c2a9c3a2cb86e280a673796e746865746963','hex');
  assert.throws(()=>decodeReply(encodeSynthetic(reply),shape),{name:'offsets-bounds'});
});
function comparePhysicalColumns(reply: DecodedReply, expected: NonNullable<ReturnType<typeof loadFixtureManifest>['fixtures'][number]['expected']>): void {
  assert.equal(reply.columns.length,expected.descriptors.length);
  reply.columns.forEach((column,index) => {
    const descriptor = expected.descriptors[index];
    assert.equal(column.descriptor.dataBuffer,descriptor.data_buffer);
    assert.equal(column.descriptor.auxBuffer,descriptor.aux_buffer);
    assert.equal(column.descriptor.stateBuffer,descriptor.state_buffer);
    if (descriptor.data_buffer !== NO_INDEX) {
      const hex = expected.buffers[descriptor.data_buffer];
      if (column.data) samePhysicalBytes(column.data,hex);
      else if (column.bytes) samePhysicalBytes(column.bytes,hex);
      else if (column.lo && column.hi) {
        const bytes = Buffer.from(hex,'hex');
        samePhysicalBytes(column.lo,bytes.subarray(0,bytes.length/2).toString('hex'));
        samePhysicalBytes(column.hi,bytes.subarray(bytes.length/2).toString('hex'));
      }
    }
    if (column.offsets) samePhysicalBytes(column.offsets,expected.buffers[descriptor.aux_buffer]);
    let stateBuffer = descriptor.state_buffer;
    for (let layer=0;layer<4;layer++) {
      if (!(descriptor.state_layers & (1<<layer))) continue;
      const state = column.states[layer];
      if (state?.data) samePhysicalBytes(state.data,expected.buffers[stateBuffer]);
      stateBuffer++;
    }
  });
}

function gameScoreColorPiece() {
  const manifest=loadFixtureManifest(resolve(folder!));
  const fixture=manifest.fixtures.find(f=>f.name==='c12-2-game-score-refusal')!;
  assert.ok(fixture?.expected,'Rust-written game-score fixture');
  const bytes=readFileSync(resolve(folder!,fixture.file));
  const buffer=bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength);
  const shape=manifest.shapes.get(fixture.shapeId)!;
  const decode=(buffer:ArrayBuffer)=>decodeReply(buffer,shape,{bounds:fixture.bounds},manifest.shapes);
  const decoded=decode(buffer);
  const color=shape.names.indexOf('identity.$item.value.color');
  assert.equal(shape.columns[color].fixedListLength,3n);
  const child=decoded.columns[color].descriptor.childFirst;
  const sections:NonNullable<SyntheticReply['extraSections']>=[],view=new DataView(buffer);
  for(let i=0;i<view.getUint32(60,true);i++) {
    const p=128+i*24,kind=view.getUint32(p,true);
    if([1,2,3,8,9].includes(kind)) continue;
    const start=Number(view.getBigUint64(p+8,true)),length=Number(view.getBigUint64(p+16,true));
    sections.push({kind,count:view.getUint32(p+4,true),bytes:new Uint8Array(buffer,start,length)});
  }
  return (length:number,first:bigint,total:bigint,final:boolean)=>{
    const extent=new Uint8Array(40),e=new DataView(extent.buffer);
    e.setUint32(0,color,true);e.setUint8(4,1);e.setUint8(5,Number(final));
    e.setBigUint64(16,first,true);e.setBigUint64(24,first+BigInt(length),true);e.setBigUint64(32,total,true);
    const reply:SyntheticReply={shapeId:fixture.shapeId,shapeVersion:decoded.header.shapeVersion,
      replyKind:decoded.header.replyKind,unitKind:decoded.header.unitKind,rowCount:decoded.header.rowCount,
      unitStart:decoded.header.unitStart,totalCount:decoded.header.totalCount,bounds:fixture.bounds,
      descriptors:decoded.columns.map(c=>({...c.descriptor})),topLevel:[...shape.topLevel],
      buffers:fixture.expected!.buffers.map(hex=>Buffer.from(hex,'hex')),
      extraSections:[...sections,{kind:8,count:1,bytes:extent}]};
    reply.buffers[reply.descriptors[color].auxBuffer!]=u64([0n,BigInt(length),BigInt(length+3),BigInt(length+6)]);
    reply.descriptors[child].length=BigInt(length+6);
    const data=reply.descriptors[child].dataBuffer!;
    reply.buffers[data]=reply.buffers[data].subarray(0,length+6);
    return decode(encodeSynthetic(reply));
  };
}

test('registered fixed-length game-score colour accepts first and final slices from the Rust fixture', {
  skip:!folder && 'Set H4R_WIRE_FIXTURES to the output directory of wire-fixtures.',
}, ()=>{
  const piece=gameScoreColorPiece();
  for(const [length,first,final] of [[1,0n,false],[2,1n,true]] as const) {
    const decoded=piece(length,first,3n,final);
    assert.equal(decoded.extents[0].total,3n);
    const child=decoded.columns[decoded.extents[0].column].descriptor.childFirst;
    assert.equal(decoded.originalIdentity(child,0).elements.at(-1),first);
  }
});

test('registered fixed-length game-score colour refuses incompatible totals from the Rust fixture', {
  skip:!folder && 'Set H4R_WIRE_FIXTURES to the output directory of wire-fixtures.',
}, ()=>{
  const piece=gameScoreColorPiece();
  for(const total of [4n,5n,0xffff_ffff_ffff_ffffn])
    assert.throws(()=>piece(3,1n,total,total===4n),{name:'descriptor-length'});
});

test('Rust-written Appendix C fixture bytes decode through the generated shape contracts', {
  skip: !folder && 'Set H4R_WIRE_FIXTURES to the output directory of wire-fixtures.',
}, async context => {
  const manifest = loadFixtureManifest(resolve(folder!));
  const defaults={encoding:0,stateLayers:0,paramA:0,paramB:0,childFirst:NO_INDEX,childCount:0,gateColumn:0xffff,gateValue:0};
  const compactShapes=new Map([...manifest.shapes].map(([id,shape])=>[id,{...shape,columns:shape.columns.map(column=>{
    const descriptor:ColumnSpec['descriptor']={nameId:column.descriptor.nameId,logicalType:column.descriptor.logicalType};
    for(const key of Object.keys(defaults) as (keyof typeof defaults)[])
      if(column.descriptor[key]!==defaults[key]) descriptor[key]=column.descriptor[key];
    return {descriptor,staticEnum:column.staticEnum,dictionaryKind:column.dictionaryKind,fixedListLength:column.fixedListLength};
  })}]));
  const generatedPath = new URL('../generated/wire.ts',import.meta.url);
  const generated = (await import(generatedPath.href)) as {
      readWire:(buffer:ArrayBuffer,id:number,expected:{bounds?:typeof manifest.fixtures[number]['bounds']})=>DecodedReply;
      WIRE_SHAPES_BY_ID:ReadonlyMap<number,unknown>};
  // Reference/conformance contracts live in the Rust fixture export only.
  for (const [id,shape] of manifest.shapes) {
    if (![8000,8001,9000].includes(id)) assert.ok(generated.WIRE_SHAPES_BY_ID.has(id),shape.name);
  }
  for (const fixture of manifest.fixtures) {
    await context.test(fixture.name, () => {
      const bytes = readFileSync(resolve(folder!,fixture.file));
      const buffer = bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength);
      const shape = manifest.shapes.get(fixture.shapeId)!;
      const read = () => generated.WIRE_SHAPES_BY_ID.has(fixture.shapeId)
        ? generated.readWire(buffer,fixture.shapeId,{bounds:fixture.bounds})
        : decodeReply(buffer,shape,{bounds:fixture.bounds},manifest.shapes);
      const readCompact=()=>decodeReply(buffer,compactShapes.get(fixture.shapeId)!,{bounds:fixture.bounds},compactShapes);
      if (fixture.expectedError) {
        assert.throws(read,(error:unknown) => error instanceof WireError && error.code === fixture.expectedError,
          `Expected named refusal ${fixture.expectedError}`);
        assert.throws(readCompact,(error:unknown)=>error instanceof WireError && error.code===fixture.expectedError);
        return;
      }
      const reply = read();
      assert.deepEqual(reply.header,readCompact().header);
      assert.deepEqual(reply.header,decodeReply(buffer,shape,{bounds:fixture.bounds},manifest.shapes).header);
      assert.equal(reply.header.shapeId,fixture.shapeId);
      assert.equal(reply.header.shapeVersion,shape.version);
      assert.equal(reply.header.unitEnd-reply.header.unitStart,reply.header.rowCount);
      assert.deepEqual(reply.bounds,fixture.bounds);
      if (fixture.expected) {
        comparePhysicalColumns(reply,fixture.expected);
        const names = {unit_start:'unitStart',unit_end:'unitEnd',row_count:'rowCount',total_count:'totalCount',
          diagnostic_total:'diagnosticTotal',query_id:'queryId',revision:'revision',generation:'generation'} as const;
        for (const [rust,ts] of Object.entries(names)) {
          assert.equal(reply.header[ts as typeof names[keyof typeof names]],BigInt(fixture.expected.header[rust] as string));
        }
        if (fixture.expected.embeddedBuffers) {
          assert.ok(reply.diagnostics);
          assert.equal(reply.diagnostics.columns.length,14);
        }
        for (const occurrence of fixture.expected.occurrences ?? []) {
          const original = reply.originalIdentity(occurrence.column,Number(occurrence.index));
          assert.equal(original.row,BigInt(occurrence.path[0].index));
          assert.deepEqual(original.elements,occurrence.path.slice(1).map(p=>BigInt(p.index)));
        }
      }
      if (fixture.name === 'c12-1-totals-diagnostics') {
        assert.deepEqual(reply.topLevel.map(c=>c.value(0)),[12n,3n,1n]);
        assert.equal(buffer.byteLength,2384);
        assert.equal(reply.diagnostics!.header.totalLength,1832n);
      }
      if (fixture.name === 'all-constructs-exact-values-states-unions') {
        checkConformanceText(reply.columns[shape.names.indexOf('text')]);
      }
      if (shape.name.includes('chart') || shape.id === 201) {
        for (const name of ['runs.$item.first','runs.$item.last']) {
          const column = reply.columns.find(c=>shape.names[c.descriptor.nameId]===name);
          if (column) assert.ok(column.data instanceof Int32Array,'Chart hours are native signed i32 views.');
        }
        assert.ok(shape.names.includes('directory'),'Every chart page registers its directory column.');
      }
      for (const [key,value] of Object.entries(fixture.header ?? {})) {
        assert.equal(reply.header[key as 'unitStart'|'unitEnd'|'rowCount'|'totalCount'],BigInt(value!));
      }
      for (const column of reply.columns) {
        for (const view of [column.data,column.lo,column.hi,column.offsets,column.bytes]) {
          if (view) assert.equal(view.buffer,buffer,'Every physical column must borrow the received buffer.');
        }
      }
      for (const expected of fixture.assertions ?? []) {
        const index = shape.names.indexOf(expected.column);
        assert.ok(index >= 0,`unknown assertion column ${expected.column}`);
        const column = reply.columns.find(c=>c.descriptor.nameId===index)!;
        assert.ok(column);
        if (expected.value !== undefined) {
          const actual = column.value(expected.index);
          assert.equal(typeof actual === 'bigint' ? actual.toString() : actual,expected.value);
        }
        if (expected.state) assert.deepEqual(column.state(expected.index).layers.map(x=>x===null?null:x.toString()),expected.state);
      }
    });
  }
  assert.ok(manifest.dictionaryFixtures.length > 0,'Rust must export the shared H4RD corpus.');
  for (const fixture of manifest.dictionaryFixtures) {
    await context.test(fixture.name, () => {
      const bytes = readFileSync(resolve(folder!,fixture.file));
      const buffer = bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength);
      const header = new DataView(buffer);
      const reference = {identity:new Uint8Array(32),shardIdentity:new Uint8Array(Buffer.from(fixture.identity,'hex')),
        firstEntry:header.getBigUint64(8,true),entryCount:header.getBigUint64(16,true),
        kind:header.getUint16(6,true),flags:0};
      const read = () => decodeDictionaryShard(buffer,reference);
      if (fixture.expectedError) {
        assert.throws(read,(error:unknown)=>error instanceof WireError && error.code===fixture.expectedError);
        return;
      }
      const shard = read();
      for (let code=0;code<fixture.expectedEntries.length;code++) {
        const entry = shard.entry(BigInt(code));
        assert.equal(entry.buffer,buffer,'Dictionary entries borrow the received shard.');
        samePhysicalBytes(entry,fixture.expectedEntries[code]);
      }
      for (const extent of fixture.byteExtents) {
        const entry = shard.entry(BigInt(extent.code));
        const first = BigInt(extent.first),end = BigInt(extent.end);
        const piece = entry.subarray(Number(first),Number(end));
        assert.equal(validateDictionaryExtent({first,end,total:BigInt(entry.length),
          final:end===BigInt(entry.length),bytes:piece}).bytes,piece);
        assert.deepEqual(Buffer.from(piece),Buffer.from(fixture.expectedEntries[Number(extent.code)],'hex').subarray(Number(first),Number(end)));
      }
    });
  }
});
