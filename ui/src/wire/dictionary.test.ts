import assert from 'node:assert/strict';
import test from 'node:test';
import { blake3, decodeDictionaryPage, decodeDictionaryShard, validateDictionaryExtent } from './dictionary.ts';
import type { DictionaryReference } from './runtime.ts';

function shard(entries: Uint8Array[], firstEntry: bigint, kind=0) {
  const pageLength=8+(entries.length+1)*8+entries.reduce((n,e)=>n+e.length,0);
  const buffer=new ArrayBuffer(56+pageLength),view=new DataView(buffer),bytes=new Uint8Array(buffer);
  bytes.set([72,52,82,68]);view.setUint16(4,1,true);view.setUint16(6,kind,true);
  view.setBigUint64(8,firstEntry,true);view.setBigUint64(16,BigInt(entries.length),true);view.setBigUint64(24,1n,true);
  view.setBigUint64(32,firstEntry,true);view.setBigUint64(40,56n,true);view.setBigUint64(48,BigInt(pageLength),true);
  view.setBigUint64(56,BigInt(entries.length),true);
  let offset=0;view.setBigUint64(64,0n,true);
  entries.forEach((entry,i)=>{
    bytes.set(entry,56+8+(entries.length+1)*8+offset);offset+=entry.length;
    view.setBigUint64(64+(i+1)*8,BigInt(offset),true);
  });
  const reference:DictionaryReference={identity:new Uint8Array(32),shardIdentity:blake3(bytes),
    firstEntry,entryCount:BigInt(entries.length),kind,flags:0};
  return {buffer,reference};
}
test('dictionary canonical identity remains exact beyond u32 and at the last u64 entry',()=>{
  for(const first of [4_380_000_000n,0xffff_ffff_ffff_ffffn]) {
    const {buffer,reference}=shard([new Uint8Array([0,255,19])],first);
    const decoded=decodeDictionaryShard(buffer,reference);
    assert.equal(decoded.firstEntry,first);
    assert.equal(decoded.entry(0n).buffer,buffer);
    assert.deepEqual(decoded.pages[0].entry(first),new Uint8Array([0,255,19]));
  }
});
test('dictionary text supports known empty entries and refuses malformed UTF8',()=>{
  const {buffer,reference}=shard([new Uint8Array(),new TextEncoder().encode('é∅')],100n,1);
  const decoded=decodeDictionaryShard(buffer,reference);
  assert.equal(decoded.entry(0n).length,0);
  assert.equal(new TextDecoder().decode(decoded.entry(1n)),'é∅');
  const bad=shard([new Uint8Array([255])],100n,1);
  assert.throws(()=>decodeDictionaryShard(bad.buffer,bad.reference),{name:'invalid-utf8'});
});
test('content address, reference flags, page offsets, and empty pages are validated by name',()=>{
  const {buffer,reference}=shard([new Uint8Array([1])],0n);
  assert.throws(()=>decodeDictionaryShard(buffer,{...reference,flags:1}),{name:'dictionary-reference'});
  assert.throws(()=>decodeDictionaryShard(buffer,{...reference,shardIdentity:new Uint8Array(32)}),{name:'dictionary-identity'});
  new DataView(buffer).setBigUint64(40,55n,true);
  reference.shardIdentity=blake3(new Uint8Array(buffer));
  assert.throws(()=>decodeDictionaryShard(buffer,reference),{name:'dictionary-page'});
  assert.throws(()=>decodeDictionaryPage(new ArrayBuffer(16),0n),{name:'dictionary-page'});
});
test('entry byte extents preserve oversized data positions without manufacturing completeness',()=>{
  const extent={first:4_380_000_000n,end:4_380_000_003n,total:4_380_000_009n,final:false,bytes:new Uint8Array([0,255,2])};
  assert.equal(validateDictionaryExtent(extent),extent);
  assert.throws(()=>validateDictionaryExtent({...extent,final:true}),{name:'dictionary-extents'});
  assert.throws(()=>validateDictionaryExtent({...extent,end:extent.end+1n}),{name:'dictionary-extents'});
});
