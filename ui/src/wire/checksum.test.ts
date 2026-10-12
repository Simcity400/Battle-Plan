import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { xxh3 } from './checksum.ts';
import { blake3 } from './dictionary.ts';
const hex=(bytes:Uint8Array)=>Buffer.from(bytes).toString('hex');
test('checksum lane multiplication writes limbs without allocating a pair for every lane',()=>{
  const source=readFileSync(new URL('./checksum.ts',import.meta.url),'utf8');
  const multiply=source.slice(source.indexOf('const multiply ='),source.indexOf('const stripe ='));
  assert.doesNotMatch(multiply,/return\s*\[/);
});
test('seedless xxh3 agrees with independent reference vectors across all dispatch boundaries',()=>{
  const vectors:[number,string][]=[[0,'2d06800538d394c2'],[1,'4a4139caf4136257'],[3,'505118c313121c0e'],
    [4,'3f6889ef166b4ad1'],[8,'93f1200f9be82671'],[9,'05b2fd4d97edcdae'],[16,'2e4359e2a04a5e32'],
    [17,'f60c53684bbc489d'],[128,'5c6ab7b5b2360539'],[129,'45fabbd183de7649'],
    [240,'c0796428068cd12e'],[241,'859dc8ab6dd85c7c'],[1023,'f35d4c7d308f7a40'],
    [1024,'7df7f049c0c1ad73'],[1025,'29adcfcac5492652'],[1048576,'f69cba6feb780800']];
  for(const [length,expected] of vectors){
    const padded=Uint8Array.from({length:length+7},(_,i)=>((i-3)*37+11)&255);
    assert.equal(xxh3(padded.subarray(3,3+length)).toString(16).padStart(16,'0'),expected,String(length));
  }
});
test('unkeyed BLAKE3 retains the standard content address',()=>{
  assert.equal(hex(blake3(new Uint8Array())),'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262');
  assert.equal(hex(blake3(new TextEncoder().encode('abc'))),'6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85');
  // Public BLAKE3 test vectors use byte i % 251, exercising both block and tree boundaries.
  const vectors:[number,string][]=[[63,'e9bc37a594daad83be9470df7f7b3798297c3d834ce80ba85d6e207627b7db7b'],
    [64,'4eed7141ea4a5cd4b788606bd23f46e212af9cacebacdc7d1f4c6dc7f2511b98'],
    [65,'de1e5fa0be70df6d2be8fffd0e99ceaa8eb6e8c93a63f2d8d1c30ecb6b263dee'],
    [1023,'10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11'],
    [1024,'42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7'],
    [1025,'d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444'],
    [2048,'e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a'],
    [2049,'5f4d72f40d7a5f82b15ca2b2e44b1de3c2ef86c426c95c1af0b6879522563030'],
    [16384,'f875d6646de28985646f34ee13be9a576fd515f76b5b0a26bb324735041ddde4']];
  for(const [length,expected] of vectors)
    assert.equal(hex(blake3(Uint8Array.from({length},(_,i)=>i%251))),expected,String(length));
});
