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
