import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'vite';

test('exact nuclear counters replace coarse lookups and preserve supplying dates including empty reads',async()=>{
  const vite=await createServer({configFile:false,optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,watch:null,hmr:false,ws:false},appType:'custom'});
  try {
    const {mergeNukes}=await vite.ssrLoadModule('/ui/src/widgets/nuclear-arsenal/preparedNukes.ts');
    const {validateNukesReply,nuclearModel,nukesAt}=await vite.ssrLoadModule('/ui/src/widgets/nuclear-arsenal/nuclearArsenalData.ts');
    const make=(firstHour:number,lastHour:number,value:number)=>({branchId:'1',types:[{ordinal:0,token:null,name:null,supported:true}],warnings:[],
      readHours:[firstHour],emptyReadHours:value===0?[firstHour]:[],series:[{tag:'holder',segments:[{firstHour,lastHour}],membership:null,
        runs:[{firstHour,lastHour,startsSegment:true,total:value,counts:[{ordinal:0,value}]}]}]});
    const rough=make(0,9,1),exact=make(5,5,0);
    const merged=validateNukesReply(mergeNukes([rough,exact]));
    assert.equal(nukesAt(nuclearModel(merged),'holder',5).run.total,0);
    assert.equal(nukesAt(nuclearModel(merged),'holder',6).run.total,1);
    assert.deepEqual(merged.readHours,[0,5]);assert.deepEqual(merged.emptyReadHours,[5]);
    assert.equal(rough.series[0].runs[0].lastHour,9);
    const missing={...make(5,5,1),summaries:[{tag:'holder',summary:{from:5,to:5,width:1,buckets:[null],runs:[]}}]};
    missing.series[0].runs=[];
    assert.equal(nukesAt(nuclearModel(validateNukesReply(mergeNukes([rough,missing]))),'holder',5),null);
  }finally {await vite.close();}
});

test('paged nuclear summaries join as nuclear replies, each country and its summary line once',async()=>{
  const vite=await createServer({configFile:false,optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,watch:null,hmr:false,ws:false},appType:'custom'});
  try {
    await vite.ssrLoadModule('/ui/src/widgets/nuclear-arsenal/preparedNukes.ts');
    const {pageJoin}=await vite.ssrLoadModule('/ui/src/dashboard/pageJoins.ts');
    const {validateNukesReply}=await vite.ssrLoadModule('/ui/src/widgets/nuclear-arsenal/nuclearArsenalData.ts');
    const summary={from:0,to:9,width:1,buckets:[null]};
    const page=(tags:string[],cursor:string|null,read:number)=>({branchId:'1',types:[{ordinal:3,token:null,name:null,supported:false}],warnings:[],
      readHours:[read],emptyReadHours:[],coverage:{lastHour:read},cursor,summaries:tags.map(tag=>({tag,summary,parts:[]})),
      series:tags.map(tag=>({tag,segments:[{firstHour:0,lastHour:9}],membership:null,
        runs:[{firstHour:0,lastHour:9,startsSegment:true,total:-tag.length,counts:[{ordinal:3,value:-tag.length}]}]}))});
    const join=pageJoin('country-nukes');
    const joined=join([page(['mod a','mod b'],'2',4),page(['mod ccc','mod a'],null,7)]);
    const reply=validateNukesReply(joined);
    assert.deepEqual(reply.series.map((series:{tag:string})=>series.tag),['mod a','mod b','mod ccc']);
    assert.equal(reply.series[2].runs[0].counts[0].value,-7);
    assert.deepEqual(reply.types.map((type:{ordinal:number})=>type.ordinal),[3]);
    assert.deepEqual(reply.readHours,[4,7]);
    assert.deepEqual(joined.summaries.map((line:{tag:string})=>line.tag),['mod a','mod b','mod ccc']);
    assert.deepEqual(joined.coverage,{lastHour:7});assert.equal(joined.cursor,null);
  }finally {await vite.close();}
});
