import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeReply, displayText, WireError } from './runtime.ts';
import type { ShapeContract } from './runtime.ts';
import { bytesOf, encodeSynthetic, refreshChecksum, syntheticShape, u64 } from './test-support.ts';
import type { SyntheticReply } from './test-support.ts';

function read(reply: SyntheticReply, names: string[], amend?: (shape: ShapeContract) => void) {
  const shape = syntheticShape(reply, names); amend?.(shape);
  const buffer = encodeSynthetic(reply);
  return decodeReply(buffer, shape, {bounds: reply.bounds});
}
function scalar(type: number, a: number, values: ArrayBufferView): SyntheticReply {
  return {shapeId: 991, replyKind: 4, rowCount: BigInt((values as Uint8Array).length),
    descriptors: [{nameId:0,logicalType:type,paramA:a,dataBuffer:0}],topLevel:[0],buffers:[bytesOf(values)]};
}
test('exact plain integers, two-limb wide values, and float bit patterns are buffer views', () => {
  const reply: SyntheticReply = {shapeId:991,replyKind:4,rowCount:2n,
    descriptors:[{nameId:0,logicalType:2,paramA:64,dataBuffer:0},
      {nameId:1,logicalType:5,paramA:1,dataBuffer:1}, {nameId:2,logicalType:7,paramA:64,dataBuffer:2}],
    topLevel:[0,1,2],buffers:[u64([(1n<<63n)+1n,0n]),u64([5n,0n,0n,1n<<63n]),u64([0x8000000000000000n,0x7ff8000000001234n])]};
  const decoded = read(reply, ['u64','i128','float']);
  assert.equal(decoded.columns[0].data?.buffer, decoded.buffer);
  assert.equal(decoded.columns[1].lo?.buffer, decoded.buffer);
  assert.equal(decoded.columns[2].data?.buffer, decoded.buffer);
  assert.equal(decoded.columns[0].value(0), (1n<<63n)+1n);
  assert.equal(decoded.columns[1].value(0),5n);
  assert.equal(decoded.columns[1].value(1),-(1n<<127n));
  assert.ok(Object.is(decoded.columns[2].value(0),-0));
  const float = decoded.columns[2].data!;
  assert.equal(new DataView(float.buffer,float.byteOffset,float.byteLength).getBigUint64(8,true),0x7ff8000000001234n);
});
test('safe53 accepts the inclusive source bound and refuses fractional and larger values', () => {
  assert.equal(read(scalar(4,64,new Float64Array([2**53])) ,['exact']).columns[0].value(0),2**53);
  for (const value of [0.5,2**53+2,NaN,Infinity])
    assert.throws(() => read(scalar(4,64,new Float64Array([value])),['exact']), {name:'invalid-safe53'});
});
test('outer state absence hides inner states and scalar payload without inventing None', () => {
  const reply: SyntheticReply = {shapeId:992,replyKind:4,rowCount:1n,
    descriptors:[{nameId:0,logicalType:4,paramA:64,dataBuffer:0,stateLayers:5,stateWidths:0x101,stateBuffer:1}],
    topLevel:[0],buffers:[bytesOf(new Float64Array([NaN])),new Uint8Array([1]),new Uint8Array([255])]};
  const decoded=read(reply,['operand']);
  assert.deepEqual(decoded.columns[0].state(0).layers,[1n,null,null,null]);
  assert.equal(decoded.columns[0].value(0),undefined);
  reply.buffers[1][0]=0;
  assert.throws(()=>read(reply,['operand']),{name:'invalid-state-code'});
});
test('inactive union payloads do not validate synthetic dictionary codes', () => {
  const reply: SyntheticReply = {shapeId:993,replyKind:4,rowCount:1n,
    descriptors:[{nameId:0,logicalType:15,childFirst:1,childCount:3},
      {nameId:1,logicalType:12,paramA:8,dataBuffer:0,length:1n},
      {nameId:2,logicalType:12,paramA:8,dataBuffer:1,length:1n,gateColumn:0,gateValue:0},
      {nameId:3,logicalType:12,paramA:8,dataBuffer:2,length:1n,gateColumn:0,gateValue:1}],
    topLevel:[0],buffers:[new Uint8Array([0]),new Uint8Array([0]),new Uint8Array([255])]};
  const amend=(shape:ShapeContract)=>{
    shape.columns[1].staticEnum=['left','right'];shape.columns[2].staticEnum=['live'];shape.columns[3].staticEnum=['paused'];
  };
  const decoded=read(reply,['union','union.$kind','union.left','union.right'],amend);
  assert.equal(decoded.columns[2].value(0),0);
  assert.equal(decoded.columns[3].state(0).active,false);
  reply.buffers[0][0]=1;
  assert.throws(()=>read(reply,['union','union.$kind','union.left','union.right'],amend),{name:'dictionary-code'});
});
test('a run-length reply beyond u32 count is validated by physical runs and accessed lazily', () => {
  const reply: SyntheticReply={shapeId:994,replyKind:4,rowCount:4_294_967_296n,
    descriptors:[{nameId:0,logicalType:2,paramA:8,encoding:3,dataBuffer:0,auxBuffer:1}],
    topLevel:[0],buffers:[new Uint8Array([3,7]),bytesOf(new Uint32Array([0xffff_ffff,1]))]};
  const decoded=read(reply,['counter']);
  assert.equal(decoded.header.rowCount,4_294_967_296n);
  assert.equal(decoded.columns[0].value(4_294_967_295),7);
  assert.equal(decoded.columns[0].data?.buffer,decoded.buffer);
});
test('signed packed integers are decoded lazily with the physical sign bit',()=>{
  const reply:SyntheticReply={shapeId:9941,replyKind:4,rowCount:4n,
    descriptors:[{nameId:0,logicalType:3,paramA:8,paramB:3,encoding:1,dataBuffer:0}],topLevel:[0],
    buffers:[new Uint8Array([199,8])]};
  const decoded=read(reply,['packed']);
  assert.deepEqual([0,1,2,3].map(i=>decoded.columns[0].value(i)),[-1,0,3,-4]);
  assert.equal(decoded.columns[0].data?.buffer,decoded.buffer);
  reply.descriptors[0].paramB=1;reply.buffers=[new Uint8Array([5])];
  assert.deepEqual([0,1,2,3].map(i=>read(reply,['packed']).columns[0].value(i)),[-1,0,-1,0]);
});
test('run-length safe53 validates each physical value without expanding the logical rows',()=>{
  const reply:SyntheticReply={shapeId:9942,replyKind:4,rowCount:4_294_967_296n,
    descriptors:[{nameId:0,logicalType:4,paramA:64,encoding:3,dataBuffer:0,auxBuffer:1}],topLevel:[0],
    buffers:[bytesOf(new Float64Array([2**53,7])),bytesOf(new Uint32Array([0xffff_ffff,1]))]};
  const decoded=read(reply,['run-exact']);
  assert.equal(decoded.columns[0].value(4_294_967_295n),7);
  reply.buffers[0]=bytesOf(new Float64Array([2**53,0.5]));
  assert.throws(()=>read(reply,['run-exact']),{name:'invalid-safe53'});
});
test('wide run lengths preserve two-limb physical views and logical scalar access', () => {
  const reply: SyntheticReply={shapeId:995,replyKind:4,rowCount:5n,
    descriptors:[{nameId:0,logicalType:5,paramA:0,encoding:3,dataBuffer:0,auxBuffer:1}],topLevel:[0],
    buffers:[u64([9n,10n,1n,2n]),bytesOf(new Uint32Array([3,2]))]};
  const decoded=read(reply,['wide']);
  assert.equal(decoded.columns[0].lo!.length,2);
  assert.equal(decoded.columns[0].value(2),(1n<<64n)+9n);
  assert.equal(decoded.columns[0].value(4),(2n<<64n)+10n);
});
test('compressed logical counts above 2^53 preserve BigInt access and reject rounded number access',()=>{
  const runs=new Uint32Array(2_097_154).fill(0xffff_ffff);
  const count=BigInt(runs.length)*0xffff_ffffn;
  const reply:SyntheticReply={shapeId:9951,replyKind:4,rowCount:count,
    descriptors:[{nameId:0,logicalType:2,paramA:8,encoding:3,dataBuffer:0,auxBuffer:1}],topLevel:[0],
    buffers:[new Uint8Array(runs.length).fill(17),bytesOf(runs)]};
  const decoded=read(reply,['large-rle']);
  assert.ok(count>1n<<53n);
  assert.equal(decoded.columns[0].logicalLength,count);
  assert.equal(decoded.columns[0].value(count-1n),17);
  assert.deepEqual(decoded.originalIdentity(0,count-1n),{row:count-1n,elements:[]});
  assert.throws(()=>decoded.columns[0].length,{name:'size-overflow'});
  assert.throws(()=>decoded.columns[0].value(Number(count-1n)),{name:'descriptor-index'});
});
test('an arbitrary-depth graph validates without consuming the JavaScript call stack', () => {
  const depth=15000;
  const descriptors=Array.from({length:depth},(_,i)=>({nameId:i,logicalType:15,childFirst:i+1,childCount:1,
    length:i===0?undefined:1n}));
  const reply:SyntheticReply={shapeId:996,replyKind:4,rowCount:1n,descriptors:[...descriptors,
    {nameId:depth,logicalType:2,paramA:8,dataBuffer:0,length:1n}],topLevel:[0],buffers:[new Uint8Array([12])]};
  const decoded=read(reply,Array.from({length:depth+1},(_,i)=>`field${i}`));
  assert.equal(decoded.columns[depth].value(0),12);
});

test('overlapping and cyclic child ranges at 15000 descriptors refuse within a linear allocation budget', () => {
  const count = 15000;
  const descriptors = Array.from({length:count}, (_,i) => i + 1 === count
    ? {nameId:i,logicalType:2,paramA:8,dataBuffer:0,length:1n}
    : {nameId:i,logicalType:15,childFirst:i+1,childCount:1,length:i===0?undefined:1n});
  const reply:SyntheticReply = {shapeId:9961,replyKind:4,rowCount:1n,descriptors,topLevel:[0],buffers:[new Uint8Array([0])]};
  const shape = syntheticShape(reply, Array.from({length:count}, (_,i) => `field${i}`));
  for (const cyclic of [true,false]) {
    const buffer = encodeSynthetic(reply), view = new DataView(buffer);
    const start = Number(view.getBigUint64(136,true));
    for (let i=0;i<count-1;i++) {
      view.setUint32(start+i*64+20,cyclic?1:count-1,true);
      view.setUint32(start+i*64+24,cyclic?count-1:1,true);
    }
    refreshChecksum(buffer);
    const original = Array.from;
    let allocatedSlots = 0;
    Array.from = ((input: ArrayLike<unknown>, ...args: unknown[]) => {
      allocatedSlots += input.length ?? 0;
      assert.ok(allocatedSlots <= count*8, `requested ${allocatedSlots} array slots for ${count} descriptors`);
      return Reflect.apply(original, Array, [input,...args]);
    }) as typeof Array.from;
    try {
      assert.throws(() => decodeReply(buffer,shape), {name:cyclic?'descriptor-cycle':'descriptor-parent'});
      assert.ok(allocatedSlots <= count*8);
    } finally { Array.from = original; }
  }
});

test('gated compact RLE checks encoded values with invalid code active only at the final row', () => {
  const count=0xffff_ffffn;
  const reply:SyntheticReply={shapeId:9943,replyKind:4,rowCount:count,
    descriptors:[{nameId:0,logicalType:15,childFirst:1,childCount:2},
      {nameId:1,logicalType:12,paramA:8,encoding:3,dataBuffer:0,auxBuffer:1,length:count},
      {nameId:2,logicalType:12,paramA:8,encoding:3,dataBuffer:2,auxBuffer:3,length:count,gateColumn:0,gateValue:1}],
    topLevel:[0],buffers:[new Uint8Array([0,1]),bytesOf(new Uint32Array([0xffff_fffe,1])),
      new Uint8Array([255]),bytesOf(new Uint32Array([0xffff_ffff]))]};
  const shape=syntheticShape(reply,['union','union.$kind','union.payload']);
  shape.columns[1].staticEnum=['empty','payload'];shape.columns[2].staticEnum=['valid'];
  const original=BigInt;
  let conversions=0;
  globalThis.BigInt = ((value: Parameters<typeof BigInt>[0]) => {
    assert.ok(++conversions<1000,'logical RLE positions expanded'); return original(value);
  }) as typeof BigInt;
  try {
    assert.throws(()=>decodeReply(encodeSynthetic(reply),shape),{name:'dictionary-code'});
    reply.buffers[0][1]=0; // Invalid payload never becomes active.
    const decoded=decodeReply(encodeSynthetic(reply),shape);
    assert.equal(decoded.columns[2].state(count-1n).active,false);
    reply.buffers[0][1]=1;reply.buffers[2][0]=0;
    assert.equal(decodeReply(encodeSynthetic(reply),shape).columns[2].value(count-1n),0);
  } finally { globalThis.BigInt=original; }
});

test('semantic validation propagates nested state once per occurrence', () => {
  const depth=64,n=256;
  const reply:SyntheticReply={shapeId:9944,replyKind:4,rowCount:BigInt(n),topLevel:[0],
    descriptors:Array.from({length:depth},(_,i)=>({nameId:i,logicalType:15,childFirst:i+1,childCount:1,
      stateLayers:1,stateWidths:1,stateBuffer:i,length:i===0?undefined:BigInt(n)})),
    buffers:Array.from({length:depth},()=>new Uint8Array(n))};
  reply.descriptors.push({nameId:depth,logicalType:12,paramA:8,dataBuffer:depth,length:BigInt(n)});
  reply.buffers.push(new Uint8Array(n));
  const shape=syntheticShape(reply,Array.from({length:depth+1},(_,i)=>`field${i}`));
  shape.columns[depth].staticEnum=['valid'];
  const buffer=encodeSynthetic(reply),original=BigInt;
  let conversions=0;
  globalThis.BigInt=((value:Parameters<typeof BigInt>[0])=>{
    assert.ok(++conversions<2000,'repeated ancestor occurrence work');return original(value);
  }) as typeof BigInt;
  try { assert.equal(decodeReply(buffer,shape).columns.length,depth+1); }
  finally { globalThis.BigInt=original; }
});

test('list ancestor activity intersects a shared RLE value and reaches the final active range', () => {
  const count=0xffff_ffffn;
  const reply:SyntheticReply={shapeId:9945,replyKind:4,rowCount:3n,topLevel:[0],
    descriptors:[{nameId:0,logicalType:14,paramA:64,childFirst:1,childCount:1,auxBuffer:0,
      stateLayers:1,stateWidths:1,stateBuffer:1},
      {nameId:1,logicalType:12,paramA:8,encoding:3,dataBuffer:2,auxBuffer:3,length:count}],
    buffers:[u64([0n,count/3n,count/3n*2n,count]),new Uint8Array([0,1,0]),
      new Uint8Array([0]),bytesOf(new Uint32Array([0xffff_ffff]))]};
  const shape=syntheticShape(reply,['list','list.$item']);shape.columns[1].staticEnum=['valid'];
  const original=BigInt;let conversions=0;
  globalThis.BigInt=((value:Parameters<typeof BigInt>[0])=>{
    assert.ok(++conversions<1000,'expanded inactive ancestor list');return original(value);
  }) as typeof BigInt;
  try {
    const decoded=decodeReply(encodeSynthetic(reply),shape);
    assert.equal(decoded.columns[1].value(count-1n),0);
    assert.equal(decoded.columns[1].state(count/3n).present,false);
    reply.buffers[1]=new Uint8Array([1,1,0]);reply.buffers[2][0]=255;
    assert.throws(()=>decodeReply(encodeSynthetic(reply),shape),{name:'dictionary-code'});
  } finally { globalThis.BigInt=original; }
});

test('list offsets retain exact child counts across the local number boundary', () => {
  for(const count of [BigInt(Number.MAX_SAFE_INTEGER),1n<<53n,(1n<<53n)+1n,0xffff_ffff_ffff_fffen]) {
    const reply:SyntheticReply={shapeId:9946,replyKind:4,rowCount:1n,topLevel:[0],
      descriptors:[{nameId:0,logicalType:14,paramA:64,childFirst:1,childCount:1,auxBuffer:0},
        {nameId:1,logicalType:15,length:count}],buffers:[u64([0n,count])]};
    const shape=syntheticShape(reply,['list','list.$item']);
    const decoded=decodeReply(encodeSynthetic(reply),shape);
    assert.equal(decoded.columns[1].logicalLength,count);
    assert.deepEqual(decoded.columns[0].value(0),{first:0n,end:count});
    reply.buffers[0]=u64([0n,count-1n]);
    assert.throws(()=>decodeReply(encodeSynthetic(reply),shape),{name:'offsets-bounds'});
  }
});

function fixedListReply(lengths: bigint[], extents: {index: number; first: bigint; end: bigint; total: bigint; final: boolean}[]) {
  const offsets=[0n];
  for(const length of lengths) offsets.push(offsets.at(-1)!+length);
  const bytes=new Uint8Array(extents.length*40),view=new DataView(bytes.buffer);
  extents.forEach((extent,i)=>{
    const p=i*40;
    view.setUint8(p+4,1);view.setUint8(p+5,Number(extent.final));
    view.setBigUint64(p+8,BigInt(extent.index),true);view.setBigUint64(p+16,extent.first,true);
    view.setBigUint64(p+24,extent.end,true);view.setBigUint64(p+32,extent.total,true);
  });
  const count=offsets.at(-1)!;
  const reply:SyntheticReply={shapeId:9947,replyKind:4,rowCount:BigInt(lengths.length),topLevel:[0],
    descriptors:[{nameId:0,logicalType:14,paramA:64,childFirst:1,childCount:1,auxBuffer:0},
      {nameId:1,logicalType:2,paramA:8,dataBuffer:1,length:count}],
    buffers:[u64(offsets),new Uint8Array(Number(count))],
    extraSections:extents.length?[{kind:8,count:extents.length,bytes}]:[]};
  const shape=syntheticShape(reply,['list','list.$item']);shape.columns[0].fixedListLength=3n;
  return {reply,shape};
}

test('fixed-length lists accept first and final slices by occurrence alongside complete values', () => {
  const {reply,shape}=fixedListReply([1n,3n,2n],[
    {index:2,first:1n,end:3n,total:3n,final:true},
    {index:0,first:0n,end:1n,total:3n,final:false},
  ]);
  const decoded=decodeReply(encodeSynthetic(reply),shape);
  assert.deepEqual([0,1,2].map(i=>decoded.columns[0].value(i)),[
    {first:0n,end:1n},{first:1n,end:4n},{first:4n,end:6n},
  ]);
  assert.deepEqual(decoded.originalIdentity(1,5),{row:2n,elements:[2n]});
  // Slice structure stays mandatory, even when the complete length matches.
  reply.buffers[0]=u64([0n,2n,4n,6n]);
  assert.throws(()=>decodeReply(encodeSynthetic(reply),shape),{name:'invalid-extent'});
});

test('fixed-length lists refuse incompatible totals even when the local slice has the fixed length', () => {
  for(const total of [4n,5n,0xffff_ffff_ffff_ffffn]) {
    const {reply,shape}=fixedListReply([3n],[{index:0,first:1n,end:4n,total,final:total===4n}]);
    assert.throws(()=>decodeReply(encodeSynthetic(reply),shape),{name:'descriptor-length'});
  }
  const complete=fixedListReply([3n],[]);
  assert.equal(decodeReply(encodeSynthetic(complete.reply),complete.shape).columns[1].length,3);
  const short=fixedListReply([2n],[]);
  assert.throws(()=>decodeReply(encodeSynthetic(short.reply),short.shape),{name:'descriptor-length'});
});

test('sparse runtime contracts apply shared shape defaults without producer buffer metadata', () => {
  const reply=scalar(2,8,new Uint8Array([7]));
  const shape=syntheticShape(reply,['value']);
  shape.columns=[{descriptor:{nameId:0,logicalType:2,paramA:8}}];
  assert.equal(decodeReply(encodeSynthetic(reply),shape).columns[0].value(0),7);
  shape.columns[0].descriptor.encoding=1;
  assert.throws(()=>decodeReply(encodeSynthetic(reply),shape),{name:'shape-mismatch'});
});

test('text byte extents validate split scalars at first, middle and final boundaries', () => {
  function piece(bytes:Uint8Array, first:number, total:number, final:boolean) {
    const extent=new Uint8Array(40),v=new DataView(extent.buffer);
    v.setUint8(4,2);v.setUint8(5,Number(final));
    v.setBigUint64(16,BigInt(first),true);v.setBigUint64(24,BigInt(first+bytes.length),true);
    v.setBigUint64(32,BigInt(total),true);
    return read({shapeId:9991,replyKind:4,rowCount:1n,
      descriptors:[{nameId:0,logicalType:11,auxBuffer:0,dataBuffer:1}],topLevel:[0],
      buffers:[u64([0n,BigInt(bytes.length)]),bytes],extraSections:[{kind:8,count:1,bytes:extent}]},['text']);
  }
  for(const text of ['\u00e9','\u2205','\u{1f600}']) {
    const bytes=new TextEncoder().encode(text);
    for(let cut=1;cut<bytes.length;cut++) {
      assert.deepEqual(piece(bytes.subarray(0,cut),0,bytes.length,false).columns[0].value(0),bytes.subarray(0,cut));
      assert.deepEqual(piece(bytes.subarray(cut),cut,bytes.length,true).columns[0].value(0),bytes.subarray(cut));
    }
    for(let i=1;i<bytes.length-1;i++) piece(bytes.subarray(i,i+1),i,bytes.length,false);
    assert.equal(displayText(piece(bytes,0,bytes.length,true).columns[0].value(0) as Uint8Array),text);
    const doubled=new TextEncoder().encode(text+text);
    piece(doubled.subarray(1,bytes.length+1),1,doubled.length,false);
  }
  for(const [bytes,first,total,final] of [
    [[0x80],0,2,false], [[0xe2,0x41],0,3,false], [[0xf0,0x80],0,4,false],
    [[0x80,0x80,0x80,0x80],1,5,true], [[0xc3],1,2,true], [[0xff],1,3,false],
    [[0xf0],0,2,false], [[0xed,0xa0],0,3,false],
  ] as const) assert.throws(()=>piece(new Uint8Array(bytes),first,total,final),{name:'invalid-utf8'});
});

test('display text preserves BOM-only, leading BOM and ordinary multibyte values', () => {
  for(const value of ['\ufeff','\ufeffx','\u00e9\u2205\u{1f600}'])
    assert.equal(displayText(new TextEncoder().encode(value)),value);
});
test('nested child identity inverts offsets and incorporates each extent coordinate', () => {
  const extentBytes=new Uint8Array(80), extent=new DataView(extentBytes.buffer);
  for(const [p,column,index,first,end,total] of [[0,0,1,20,22,25],[40,2,5,8,12,15]]) {
    extent.setUint32(p,column,true);extent.setUint8(p+4,1);
    extent.setBigUint64(p+8,BigInt(index),true);extent.setBigUint64(p+16,BigInt(first),true);
    extent.setBigUint64(p+24,BigInt(end),true);extent.setBigUint64(p+32,BigInt(total),true);
  }
  const reply:SyntheticReply={shapeId:997,replyKind:2,rowCount:2n,unitStart:40n,totalCount:42n,
    descriptors:[{nameId:0,logicalType:14,paramA:64,childFirst:1,childCount:1,auxBuffer:0},
      {nameId:1,logicalType:14,paramA:64,childFirst:2,childCount:1,auxBuffer:1,length:4n},
      {nameId:2,logicalType:14,paramA:64,childFirst:3,childCount:1,auxBuffer:2,length:8n},
      {nameId:3,logicalType:2,paramA:8,dataBuffer:3,length:4n}],
    topLevel:[0],buffers:[u64([0n,2n,4n]),u64([0n,2n,4n,6n,8n]),u64([0n,0n,0n,0n,0n,0n,4n,4n,4n]),new Uint8Array([1,2,3,4])],
    extraSections:[{kind:8,count:2,bytes:extentBytes}]};
  const decoded=read(reply,['runs','runs.$item','countries','country']);
  assert.deepEqual(decoded.originalIdentity(3,2),{row:41n,elements:[20n,1n,10n]});
});
test('an orphan descriptor is refused by name even when the contract repeats the malformed graph',()=>{
  const reply:SyntheticReply={shapeId:998,replyKind:4,rowCount:1n,
    descriptors:[{nameId:0,logicalType:2,paramA:8,dataBuffer:0},
      {nameId:1,logicalType:2,paramA:8,dataBuffer:1,length:1n}],topLevel:[0],buffers:[new Uint8Array([0]),new Uint8Array([0])]};
  assert.throws(()=>read(reply,['root','orphan']),{name:'descriptor-parent'});
});
test('bytes remain arbitrary bytes; text validates before display; malformed bodies carry the query id',()=>{
  const reply:SyntheticReply={shapeId:999,replyKind:4,rowCount:1n,
    descriptors:[{nameId:0,logicalType:10,auxBuffer:0,dataBuffer:1}],topLevel:[0],
    buffers:[u64([0n,1n]),new Uint8Array([255])]};
  assert.deepEqual(read(reply,['bytes']).columns[0].value(0),new Uint8Array([255]));
  reply.descriptors[0].logicalType=11;
  assert.throws(()=>read(reply,['text']),{name:'invalid-utf8'});
  reply.buffers[1]=new TextEncoder().encode('z');
  assert.equal(displayText(read(reply,['text']).columns[0].value(0) as Uint8Array),'z');
  const buffer=encodeSynthetic(reply);new Uint8Array(buffer)[buffer.byteLength-1]^=1;
  assert.throws(()=>decodeReply(buffer,syntheticShape(reply,['text'])),(error:unknown)=>error instanceof WireError &&
    error.code==='checksum-mismatch' && error.queryId===1n);
  refreshChecksum(buffer);
});
