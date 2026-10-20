import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeTerritoryReads } from './preparedTerritory.ts';
import type { TerritoryReply } from '../../generated/dashboard';
import type { MeasureSummaries } from '../country-measure/preparedMeasureData';

test('dense envelopes do not become exact territory amounts and opaque group tags remain countries',()=>{
  const reply:TerritoryReply={branchId:'1',areaUnit:null,areaReason:'No map',warnings:[],worldTotals:[],series:[{
    tag:'group',segments:[{firstHour:100,lastHour:200}],membership:null,runs:[{firstHour:100,lastHour:100,startsSegment:true,values:[3,null,null,null,3,null,null,null, null, null, null, null]}],
  }]};
  const summaries={branchId:'1',measure:'states',decimals:0,warnings:[],series:[{tag:'group',marks:null,summary:{from:100,to:200,width:1,buckets:[{
    firstHour:100,lastHour:200,firstValue:3,lastValue:9,lastValueHour:200,minHour:100,minValue:3,maxHour:180,maxValue:900000,firstBreak:true,hasBreak:true,
  }]}}]} as MeasureSummaries;
  const result=mergeTerritoryReads(reply,undefined,summaries,0)!;
  assert.equal(result.series[0].tag,'group');
  assert.equal(result.series[0].runs[0].values[0],3);
  assert.equal(result.series[0].runs.length,1);
});

test('reached canonical world and country runs shared by head and hover occur once',()=>{
  const run={firstHour:100,lastHour:200,startsSegment:true,values:[3,4,5,null,3,4,5,null, null, null, null, null]};
  const reply:TerritoryReply={branchId:'1',areaUnit:null,areaReason:'No map',warnings:[],worldTotals:[run],series:[{tag:'opaque / realm',segments:[{firstHour:100,lastHour:200}],membership:null,runs:[run]}]};
  const result=mergeTerritoryReads(reply,reply,undefined,0)!;
  assert.deepEqual(result,reply);
});

test('dense territory labels keep the exact endpoint facts without treating maxima as amounts',()=>{
  const headRun={firstHour:100,lastHour:110,startsSegment:true,values:[3,4,5,null,3,4,5,null, null, null, null, null]};
  const endRun={firstHour:190,lastHour:200,startsSegment:false,values:[9,10,11,null,9,10,11,null, null, null, null, null]};
  const reply:TerritoryReply={branchId:'1',areaUnit:null,areaReason:null,warnings:[],worldTotals:[],series:[{tag:'opaque realm',segments:[{firstHour:100,lastHour:200}],membership:null,runs:[headRun]}]};
  const summaries={branchId:'1',measure:'states',decimals:0,warnings:[],series:[{tag:'opaque realm',marks:null,endRuns:[endRun],summary:{from:100,to:200,width:1,buckets:[{firstHour:100,lastHour:200,firstValue:3,lastValue:9,lastValueHour:190,minHour:100,minValue:3,maxHour:150,maxValue:999,firstBreak:true,hasBreak:false}]}}]};
  const result=mergeTerritoryReads(reply,undefined,summaries,0)!;
  assert.deepEqual(result.series[0].runs,[headRun,endRun]);
});

test('rough tail geometry never becomes an exact territory amount',()=>{
  const headRun={firstHour:100,lastHour:100,startsSegment:true,values:[3,null,null,null,null,null,null,null, null, null, null, null]};
  const head:TerritoryReply={branchId:'1',areaUnit:null,areaReason:null,warnings:[],worldTotals:[],series:[{tag:'tail country',segments:[],membership:null,runs:[headRun]}]};
  const summary={from:100,to:200,width:101,buckets:[],runs:[{firstHour:101,lastHour:200,value:999,breakBefore:true}],approximate:[{firstHour:101,lastHour:200}]};
  const reply={branchId:'1',measure:'states',decimals:0,warnings:[],series:[{tag:'tail country',summary,marks:null}]};
  assert.deepEqual(mergeTerritoryReads(head,undefined,reply,0)!.series[0].runs,[headRun]);
  assert.equal(reply.series[0].summary,summary,'the shared drawing retains rough spans and geometry');
});

test('an exact reached holder replaces the rough identity at only that hour',()=>{
  const run={firstHour:150,lastHour:150,startsSegment:true,values:[7,null,null,null,null,null,null,null, null, null, null, null]};
  const head:TerritoryReply={branchId:'1',areaUnit:null,areaReason:null,warnings:[],worldTotals:[],series:[{
    tag:'opaque reused tag',segments:[{firstHour:150,lastHour:150}],identities:[{firstHour:150,lastHour:150,slot:9,countryId:77}],membership:null,runs:[run],
  }]};
  const summary={from:100,to:200,width:101,buckets:[],runs:[{firstHour:100,lastHour:200,value:999}],approximate:[{firstHour:100,lastHour:200}]};
  const reply={branchId:'1',measure:'states',decimals:0,warnings:[],series:[{tag:'opaque reused tag',summary,marks:null,
    segments:[{firstHour:100,lastHour:200}],identities:[{firstHour:100,lastHour:200,slot:8,countryId:44}]}]};
  const line=mergeTerritoryReads(head,undefined,reply,0)!.series[0];
  assert.deepEqual(line.identities,[{firstHour:100,lastHour:149,slot:8,countryId:44},
    {firstHour:150,lastHour:150,slot:9,countryId:77},{firstHour:151,lastHour:200,slot:8,countryId:44}]);
  assert.deepEqual(line.runs,[run]);
});

test('occupied exact summary endpoints use twelve columns and never borrow native owned states',()=>{
  const head:TerritoryReply={branchId:'1',areaUnit:null,areaReason:'No map',warnings:[],worldTotals:[],series:[]};
  const summary={from:100,to:101,width:2,buckets:[],runs:[{firstHour:100,lastHour:101,value:-7,breakBefore:true}]};
  const reply={branchId:'1',measure:'victoryPoints',decimals:0,warnings:[],series:[{tag:'foreign controller',summary,marks:null}]};
  const result=mergeTerritoryReads(head,undefined,reply,10)!;
  assert.equal(result.series[0].runs[0].values.length,12);
  assert.deepEqual(result.series[0].runs[0].values,[null,null,null,null,null,null,null,null,null,null,-7, null]);
});


test('occupied map area exact endpoints use the twelfth wire column',()=>{
  const head:TerritoryReply={branchId:'1',areaUnit:'map cells',areaReason:null,warnings:[],worldTotals:[],series:[]};
  const summary={from:100,to:101,width:2,buckets:[],runs:[{firstHour:100,lastHour:101,value:27,breakBefore:true}]};
  const reply={branchId:'1',measure:'area',decimals:0,warnings:[],series:[{tag:'foreign controller',summary,marks:null}]};
  const result=mergeTerritoryReads(head,undefined,reply,11)!;
  assert.deepEqual(result.series[0].runs[0].values,[null,null,null,null,null,null,null,null,null,null,null,27]);
});
