import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mergeNavalWindows, validateNavalWindow, validateNavalLossesReply, navalModel, rankNaval } from './navalLossesData.ts';
import type { NavalLossesReply } from '../../generated/dashboard';
const manifest = (): NavalLossesReply => ({branchId:'1',available:true,firstRead:100,shipFirstRead:100,convoyFirstRead:100,
  shipLastRead:130,convoyLastRead:130,shipReads:[100,130],convoyReads:[100,130],warnings:[],series:[{
    tag:'mod root',segments:[{firstHour:100,lastHour:130}],identities:[{slot:1,countryId:11,firstHour:100,lastHour:130}],membership:null,runs:[],
  }]});
const page = (firstHour: number,lastHour: number,ships: number | null,convoys: number | null): NavalLossesReply => ({...manifest(),
  shipReads:[firstHour],convoyReads:[firstHour],series:[{...manifest().series[0],runs:[{firstHour,lastHour,startsSegment:firstHour===100,ships,convoys}]}]});
test('prepared windows keep global baselines and only their supplying reads',()=>{
  const value = page(110,119,2,-3);
  assert.throws(()=>validateNavalLossesReply(value));
  assert.equal(validateNavalWindow(value),value);
  assert.throws(()=>validateNavalWindow({...value,shipReads:[99]}));
});
test('reached windows join equal native values and preserve unknown and signed corrections',()=>{
  const first=page(100,109,2,-3),second=page(110,119,2,-3),missing=page(120,129,null,-3),last=page(130,130,3,-2);
  const reply=mergeNavalWindows(manifest(),[last,first,missing,second]);
  assert.deepEqual(reply.series[0].runs,[
    {firstHour:100,lastHour:119,startsSegment:true,ships:2,convoys:-3},
    {firstHour:120,lastHour:129,startsSegment:false,ships:null,convoys:-3},
    {firstHour:130,lastHour:130,startsSegment:false,ships:3,convoys:-2},
  ]);
  assert.deepEqual(reply.shipReads,[100,110,120,130]);
  assert.equal(first.series[0].runs[0].lastHour,109);
  const model=navalModel(reply);
  assert.equal(rankNaval(model,['mod root'],110,'both',10)[0].value,-1);
  assert.equal(rankNaval(model,['mod root'],120,'both',10).length,0);
  assert.equal(rankNaval(model,['mod root'],130,'both',10)[0].value,1);
});
