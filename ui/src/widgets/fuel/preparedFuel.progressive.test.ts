import assert from 'node:assert/strict';
import test from 'node:test';
import * as React from 'react';
import { createServer } from 'vite';
import { wireSummary } from '../../charts/chartSummary.fixture.ts';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';

test('mounted fuel publishes scalar tails immediately and replaces them when preparation accepts more coverage', {timeout:5000}, async()=>{
  const vite=await createServer({configFile:false,optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,watch:null,hmr:false,ws:false},appType:'custom'});
  type Slot={value?:unknown;initialized?:boolean;deps?:readonly unknown[];cleanup?:()=>void};
  const slots:Slot[]=[];let cursor=0,effects:(()=>void)[]=[];
  const internals=(React as unknown as {__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE:{H:unknown}}).__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
  const state=(initial:unknown)=>{const slot=slots[cursor++]??={};if(!slot.initialized){slot.value=typeof initial==='function'?initial():initial;slot.initialized=true;}return slot;};
  const memo=(make:()=>unknown,deps:readonly unknown[])=>{const slot=slots[cursor++]??={};if(!slot.deps||deps.some((dep,index)=>!Object.is(dep,slot.deps![index]))){slot.deps=deps;slot.value=make();}return slot.value;};
  const dispatcher={useState(initial:unknown){const slot=state(initial);return [slot.value,(next:unknown)=>{slot.value=typeof next==='function'?next(slot.value):next;}];},
    useRef(initial:unknown){return state({current:initial}).value;},useMemo:memo,useCallback:(callback:()=>unknown,deps:readonly unknown[])=>memo(()=>callback,deps),
    useSyncExternalStore(_subscribe:unknown,snapshot:()=>unknown){cursor++;return snapshot();},
    useEffect(make:()=>void|(()=>void),deps:readonly unknown[]){const slot=slots[cursor++]??={};if(!slot.deps||deps.some((dep,index)=>!Object.is(dep,slot.deps![index]))){slot.deps=deps;effects.push(()=>{slot.cleanup?.();slot.cleanup=make()||undefined;});}}};
  const flush=()=>new Promise<void>(resolve=>setImmediate(resolve));
  let release!:()=>void;const accepted=new Promise<void>(resolve=>{release=resolve;});
  let ready=false;const calls=new Map<string,number>();let statuses=0;
  const label={key:'fixture',template:null,arguments:['NUM'],reason:'fixture'};
  const metadata={branchId:'1',tag:'tail',quantityScale:32768,gainScale:100000,dayHours:24,detailHours:720,consumerNames:[null,null,null],
    gainLabels:[label,label,label,label],transferLabels:[label,label,label],reads:[],warnings:[]};
  const client={preparedQuery:async(_source:unknown,product:string,params:{reads?:{tag:string;hour:number}[]})=>{
    const count=(calls.get(product)??0)+1;calls.set(product,count);assert.ok(count<=3,'initial and accepted publications cannot loop');
    const coverage={firstHour:0,lastHour:ready?29:1,complete:ready};
    if(product==='series-summary')return {branchId:'1',coverage,replies:[metadata],series:[{tag:'tail',measure:'fuel',summary:wireSummary({from:0,to:29,width:30,
      buckets:[],runs:[{firstHour:0,lastHour:29,value:ready?2:1,breakBefore:true}],approximate:ready?[]:[{firstHour:2,lastHour:29}]})}]};
    return {branchId:'1',coverage,replies:[metadata],states:params.reads!.map(read=>({...read,recorded:true,approximate:false,fuel:null,consumption:null,
      consumptionSegment:null,fuelRaw:ready?'2':'1',capacityRaw:'3'}))};
  },preparationStatus:async(input:object)=>{assert.ok(++statuses<=3);await accepted;return {...input,state:'ready',sections:[],bytes:'0',error:null,
    backgroundEnabled:true,coverage:{'1':{firstHour:0,lastHour:29,complete:true}}};}} as unknown as DashboardClient;
  try {
    const {usePreparedFuel}=await vite.ssrLoadModule('/ui/src/widgets/fuel/preparedFuel.ts');
    const track={source:{id:'fuel-progressive',revision:'1'},branch:{id:'1',firstHour:0,lastHour:29}} as BoundTrack;
    const render=()=>{cursor=0;effects=[];const previous=internals.H;internals.H=dispatcher;let result;try{result=usePreparedFuel(client,track,['tail'],29,true,['fuel']);}finally{internals.H=previous;}for(const effect of effects)effect();return result;};
    render().onSummaryView({from:0,to:29,width:30});render();await flush();
    let shown=render();await flush();shown=render();
    assert.equal(shown.amountsAt('tail',29).fuelRaw,'1');
    assert.deepEqual(shown.summary('tail','fuel').approximate,[{firstHour:2,lastHour:29}]);
    ready=true;release();await flush();await flush();shown=render();
    assert.equal(shown.amountsAt('tail',29).fuelRaw,'2');
    assert.deepEqual(shown.summary('tail','fuel').approximate,[]);
    assert.equal(calls.get('series-summary'),2);
    assert.ok((calls.get('series-state')??0)<=3);
  }finally{for(const slot of slots)slot?.cleanup?.();release();await vite.close();}
});
