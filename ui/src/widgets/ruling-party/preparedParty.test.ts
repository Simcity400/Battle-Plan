import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { PartyHistoryReply } from '../../generated/dashboard';
import type { WidgetProps } from '../../dashboard/definitions';
import type { DashboardClient } from '../../dashboard/client';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import { plotFrame } from '../../charts/frame.ts';
import { drawBands } from '../../charts/draw.ts';
import { bands, buildStack, rulingFirst, stackedSegments } from './partyData.ts';
import { summaryStack } from '../../charts/chartSummary.ts';

const reply = (): PartyHistoryReply => ({branchId:'1',tag:'opaque / country',decimals:5,warnings:[],ideologies:[],ideologyTypes:[],people:[],leaders:[],
  partyNames:[],partyNameRuns:[],elections:[],unobserved:[],ruling:[{token:'mod_a',firstHour:0,lastHour:2000,startsSegment:true}],
  parties:[{token:'mod_a',runs:[{firstHour:0,lastHour:2000,valueRaw:'4000000',startsSegment:true}]},
    {token:'mod_b',runs:[{firstHour:0,lastHour:2000,valueRaw:'3500000',startsSegment:true}]}]});

test('prepared party replies use the legacy overlap refusal and preserve marker glyphs', async()=> {
  const vite=await createServer({configFile:false,server:{middlewareMode:true,watch:null,hmr:false,ws:false}});
  try {
    const {validatePreparedParty,partyMarkSummaries}=await vite.ssrLoadModule('/ui/src/widgets/ruling-party/preparedParty.ts') as typeof import('./preparedParty');
    const data={coverage:{firstHour:0,lastHour:2000,complete:true},reply:reply(),summaries:[],marks:[],cursor:null,total:'0'};
    assert.equal(validatePreparedParty(data),data);
    for (const cursor of ['0', '127']) assert.throws(() => validatePreparedParty({...data,cursor}, '127'), /did not advance/);
    assert.equal(validatePreparedParty({...data,cursor:'128'}, '127').cursor, '128', 'an empty membership-filtered page may advance its raw cursor');
    assert.equal(validatePreparedParty(data, '127'), data, 'a terminal page has no continuation');
    const cold = {...data,coverage:{firstHour:2,lastHour:1,complete:false}};
    assert.equal(validatePreparedParty(cold), cold);
    for (const coverage of [null, {...cold.coverage,lastHour:0}, {...cold.coverage,complete:true}])
      assert.throws(() => validatePreparedParty({...data,coverage}), /malformed/);
    const malformed=reply();malformed.parties[0].runs.push({...malformed.parties[0].runs[0]});
    assert.throws(()=>validatePreparedParty({...data,reply:malformed}),/party history reply is malformed/);
    const marks=['ruling','election','leader'].map((field,index)=>({field,exact:false,summary:{from:0,to:100,width:100,clusters:[{firstHour:index,lastHour:index,firstOrdinal:'0',count:'1'}]}}));
    const badges=['\u21c6','\u2713','\u2654'].map((glyph,hour)=>({hour,glyph,label:`change ${hour}`}));
    assert.deepEqual(partyMarkSummaries({...data,marks},badges)?.map(mark=>mark.clusters[0].badge),badges);
    assert.deepEqual(partyMarkSummaries({...data,marks:marks.map(mark=>({...mark,exact:true}))},badges),[]);
    const {partyParityRows}=await vite.ssrLoadModule('/ui/src/widgets/ruling-party/partyParity.ts') as typeof import('./partyParity');
    const changed=reply();changed.ruling=[{token:'mod_a',firstHour:0,lastHour:2,startsSegment:true},{token:'mod_b',firstHour:3,lastHour:10,startsSegment:false}];
    assert.equal(partyParityRows(changed,[0,10],true).length,1);
    assert.deepEqual(partyParityRows(changed,[0,10],true,hour=>hour<3),[]);
  } finally {await vite.close();}
});

test('ruling-party renders bounded prepared views, masks departed owners and refuses stalled event pages',async()=> {
  type Slot={value?:unknown;deps?:readonly unknown[];cleanup?:()=>void};
  const fixture={cursor:0,slots:[] as Slot[],effects:[] as (()=>void)[],chart:undefined as TimeChartProps|undefined,
    requests:[] as {product:string;params:Record<string,unknown>;signal:AbortSignal}[],pending:[] as {signal:AbortSignal;resolve:(value:unknown)=>void}[]};
  let prepared = false, statuses = 0;
  const statusReplies: (() => void)[] = [];
  const host=globalThis as typeof globalThis & {__partyHook?:(name:string,...args:unknown[])=>unknown;__partyChart?:(props:TimeChartProps)=>void};
  host.__partyChart=props=>{fixture.chart=props;};
  host.__partyHook=(name,...args)=> {
    const slot=fixture.slots[fixture.cursor++] ??= {};
    if(name==='useMemo'||name==='useCallback'||name==='useEffect') {
      const [factory,deps]=args as [()=>unknown,readonly unknown[]];
      if(!slot.deps || slot.deps.length!==deps.length || deps.some((value,index)=>!Object.is(value,slot.deps![index]))) {
        slot.deps=[...deps];
        if(name==='useMemo')slot.value=factory();else if(name==='useCallback')slot.value=factory;
        else fixture.effects.push(()=>{slot.cleanup?.();const cleanup=factory();slot.cleanup=typeof cleanup==='function' ? cleanup as ()=>void : undefined;});
      }
      return slot.value;
    }
    if (name==='useRef') {if(!('value' in slot))slot.value={current:args[0]};return slot.value;}
    if(!('value' in slot))slot.value=typeof args[0]==='function' ? (args[0] as ()=>unknown)() : args[0];
    return [slot.value,(next:unknown)=>{slot.value=typeof next==='function' ? (next as (old:unknown)=>unknown)(slot.value) : next;}];
  };
  const replacements:Record<string,string>={
    '/dashboard/playback.ts':'export const usePlaybackHour=()=>100;',
    '/charts/TimeChart.tsx':'export default props=>{globalThis.__partyChart(props);return null;};',
    '/charts/Legend.tsx':'export default ()=>null;',
    '/charts/tokens.ts':'export const readTokens=()=>({surface:"#101820"});export const subscribeTokens=()=>()=>{};',
    '/components/ContextMenu.tsx':'export const useContextMenu=()=>({open(){},menu:null});',
  };
  const vite=await createServer({configFile:false,optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,watch:null,hmr:false,ws:false},
    plugins:[{name:'prepared-party-fixture',enforce:'pre',transform(source,id) {
      const path=id.replaceAll('\\','/'),replacement=Object.entries(replacements).find(([suffix])=>path.endsWith(suffix));
      if(replacement)return replacement[1];
      if(path.endsWith('/dashboard/contentRevision.ts'))return source.replace('return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);','return revision.snapshot();');
      // Held failures run on the same synthetic hooks, outside a widget frame.
      if(['/ruling-party/RulingPartyWidget.tsx','/ruling-party/preparedParty.ts','/ruling-party/preparedGovernments.ts','/dashboard/products.ts','/dashboard/heldFailures.ts'].some(suffix=>path.endsWith(suffix)))
        return source.replace('useContext(HeldFailuresContext)','null').replace(/\b(useMemo|useCallback|useState|useRef|useEffect)(?:<[\s\S]*?>)?\(/g,(_match,name)=>`globalThis.__partyHook(${JSON.stringify(name)}, `);
    }}]});
  const client={product:()=>{throw new Error('Party history must come from preparation.');},
    preparedQuery:async (_source:unknown,product:string,params:Record<string,unknown>,{signal}:{signal:AbortSignal})=> {
      fixture.requests.push({product,params,signal});assert.ok(fixture.requests.length <= 19, 'reached party views must not repeat requests');assert.equal(product,'party-history');
      if(params.mode==='next')return {hour:102};
      if(params.tag==='departing')return new Promise(resolve=>fixture.pending.push({signal,resolve}));
      const data=reply();data.tag=params.tag as string;
      if(params.tag==='unprepared owner') {
        data.ruling=[];
        return {coverage:{firstHour:0,lastHour:prepared ? 2000 : 99,complete:prepared},reply:data,summaries:params.mode==='summary' ? data.parties.map(party=>({token:party.token,summary:{from:params.from,to:params.to,width:params.width,
          approximate:prepared ? [] : [{firstHour:100,lastHour:104}],buckets:[100,103].map(hour=>({firstHour:hour,lastHour:hour===100 ? 102 : 104,
            firstValue:Number(party.runs[0].valueRaw)/100000,lastValue:Number(party.runs[0].valueRaw)/100000,lastValueHour:hour,minHour:hour,maxHour:hour,
            minValue:Number(party.runs[0].valueRaw)/100000,maxValue:Number(party.runs[0].valueRaw)/100000,firstBreak:hour===100,hasBreak:false}))}})) : [],marks:[],total:'0',cursor:null};
      }
      if(params.mode==='events') {data.elections=[101];data.parties.forEach(party=>{party.runs=[];});}
      return {coverage:{firstHour:0,lastHour:2000,complete:true},reply:data,summaries:params.mode==='summary' ? data.parties.map(party=>({token:party.token,summary:{from:params.from,to:params.to,width:params.width,
        runs:party.runs.map(run=>({firstHour:run.firstHour,lastHour:run.lastHour,value:Number(run.valueRaw)/100000,breakBefore:run.startsSegment})),
        buckets:Array.from({length:5},(_,index)=>({firstHour:100+index,lastHour:100+index,firstValue:Number(party.runs[0].valueRaw)/100000,lastValue:Number(party.runs[0].valueRaw)/100000,
          minHour:100+index,maxHour:100+index,lastValueHour:100+index,minValue:Number(party.runs[0].valueRaw)/100000,maxValue:Number(party.runs[0].valueRaw)/100000,firstBreak:false,hasBreak:false}))}})) : [],marks:[],total:'1',cursor:params.mode==='events' && params.from===103 ? params.cursor : params.cursor==='0' ? '128' : null};
    },
    preparationStatus:(request:object,signal:AbortSignal)=> {
      assert.ok(++statuses<=3,'each published partial view waits for the same accepted-coverage update');
      return new Promise((resolve,reject)=> {
        const aborted=()=>reject(new DOMException('cancelled','AbortError'));
        signal.addEventListener('abort',aborted,{once:true});
        statusReplies.push(()=>{signal.removeEventListener('abort',aborted);resolve({...request,state:'ready',sections:[],coverage:{'1':{firstHour:0,lastHour:2000,complete:true}},backgroundEnabled:true,bytes:'0',error:null});});
      });
    }};
  try {
    const {default:Widget}=await vite.ssrLoadModule('/ui/src/widgets/ruling-party/RulingPartyWidget.tsx');
    const props={widget:{kind:'ruling-party',trackIds:['track'],countries:{kind:'tags',tags:['opaque / country']},time:{mode:'range',firstOffset:100,lastOffset:104},options:{changes:false}},
      runtime:{client,tracks:[{id:'track',axisOffset:0,source:{id:'source',revision:'1'},branch:{id:'1',firstHour:0,lastHour:2000}}],clock:{getHour:()=>104,getSnapshot:()=>({hoursPerSecond:1})}},active:true,onDiagnostics(){}} as unknown as WidgetProps;
    const render=()=>{fixture.cursor=0;return renderToStaticMarkup(createElement(Widget,props));};
    const commit=async()=>{fixture.effects.splice(0).forEach(run=>run());await new Promise(resolve=>setImmediate(resolve));};
    render();await commit();render();fixture.chart!.onSummaryView!({from:100,to:104,width:200});render();await commit();render();
    const view=fixture.requests.find(request=>request.params.mode==='summary')!;
    assert.deepEqual([view.params.from,view.params.to,view.params.width],[100,104,200]);
    const frame=plotFrame(200,200,{left:0,right:0,top:0,bottom:0},[100,104],[0,100],false),tokens={lineWidth:{series:2}} as ChartTokens;
    const path=(draw:(ctx:CanvasRenderingContext2D)=>void)=>{const calls:unknown[][]=[];const ctx=new Proxy({},{get:(_target,key)=>(...args:unknown[])=>calls.push([key,...args]),set:()=>true}) as CanvasRenderingContext2D;draw(ctx);return calls;};
    const list=rulingFirst(bands(reply(),['#123','#456']),'mod_a');
    const preparedPath=path(ctx=>fixture.chart!.layers[0].draw(ctx,frame,tokens,{playhead:null}));
    const plainPath=path(ctx=>{for(const segment of stackedSegments(buildStack(list,5),100,104,4/frame.plotWidth/2))drawBands(ctx,frame,tokens,segment.hours,segment.bands,list.map(band=>band.color));});
    assert.deepEqual(preparedPath.map(call=>call[0]),plainPath.map(call=>call[0]));
    const resized=plotFrame(400,200,{left:0,right:0,top:0,bottom:0},[100,104],[0,100],false);
    assert.deepEqual(path(ctx=>fixture.chart!.layers[0].draw(ctx,resized,tokens,{playhead:null})),[],'a departed viewport cannot paint even exact runs');
    assert.deepEqual(preparedPath.filter(call=>call[0]==='moveTo'||call[0]==='lineTo'),plainPath.filter(call=>call[0]==='moveTo'||call[0]==='lineTo'),'exact geometry follows legacy stack paths');
    fixture.chart!.onHover!(101);render();await commit();render();
    assert.ok(fixture.requests.some(request=>request.params.mode==='events'&&request.params.field==='all'&&request.params.from===101));
    fixture.chart!.onMarkHover!({firstHour:101,lastHour:101,firstOrdinal:'0',count:'300',tag:'election'});render();await commit();
    assert.match(render(),/Next events/);
    // The widget's shared keyboard pick also reaches the chosen hour's values and events.
    fixture.chart!.onNavigateHour!('ArrowRight',100,[100,104],null);render();await commit();render();assert.equal(fixture.chart!.keyboardHour,102);
    props.widget.countries={kind:'tags',tags:['departing']};assert.match(render(),/Loading parties/);await commit();
    props.widget.countries={kind:'tags',tags:['next owner']};render();await commit();assert.ok(fixture.pending[0].signal.aborted);
    fixture.pending[0].resolve({coverage:{firstHour:0,lastHour:2000,complete:true},reply:reply(),summaries:[],marks:[],total:'0',cursor:null});await new Promise(resolve=>setImmediate(resolve));
    render();assert.equal(fixture.chart!.label,"Party popularity in next owner");
    assert.equal(fixture.requests.length,12);
    fixture.chart!.onMarkHover!({firstHour:103,lastHour:103,firstOrdinal:'0',count:'300',tag:'election'});render();await commit();
    assert.match(render(),/event cursor did not advance/);
    for (let attempt=0;attempt<3;attempt++) {await commit();assert.match(render(),/event cursor did not advance/);}
    props.active=false;render();await commit();
    assert.equal(fixture.requests.length,13,'a stalled page is refused without automatic re-requests');
    for(const slot of fixture.slots)slot.cleanup?.();fixture.slots=[];fixture.effects=[];
    const {usePreparedParty}=await vite.ssrLoadModule('/ui/src/widgets/ruling-party/preparedParty.ts') as typeof import('./preparedParty');
    const renderHook=()=>{fixture.cursor=0;return usePreparedParty(client as unknown as DashboardClient,props.runtime.tracks[0].source,'1','unprepared owner',104,null,true);};
    renderHook();await commit();let native=renderHook();
    assert.equal(native.data!.parties.length,2,'an unprepared state publishes before its preparation status settles');
    native.onSummaryView({from:100,to:104,width:2});renderHook();await commit();native=renderHook();
    assert.equal(native.error,undefined);
    assert.deepEqual(native.summary!.coverage,{firstHour:0,lastHour:99,complete:false});
    assert.deepEqual(native.summary!.summaries[0].summary.approximate,[{firstHour:100,lastHour:104}]);
    assert.deepEqual(summaryStack(native.summary!.summaries.map(line=>line.summary)).approximate,[{firstHour:100,lastHour:104}], 'the hook preserves spans for the one shared stack/drawing path');
    assert.equal(fixture.requests.length,15,'two partial view requests publish immediately without a cache or retry loop');
    native.onMarkHover({firstHour:100,lastHour:104,firstOrdinal:'0',count:'1',tag:'election'});renderHook();await commit();native=renderHook();
    const {default:EventList}=await vite.ssrLoadModule('/ui/src/widgets/ruling-party/PartyEventList.tsx') as typeof import('./PartyEventList');
    const log=renderToStaticMarkup(createElement(EventList,{prepared:native,badges:[]}));
    assert.equal((log.match(/still being prepared/g) ?? []).length,1,'a reached empty page uses the shared prepared-range note');
    prepared=true;statusReplies.splice(0).forEach(resolve=>resolve());await new Promise(resolve=>setImmediate(resolve));native=renderHook();
    assert.equal(native.summary!.coverage!.complete,true);
    assert.deepEqual(native.summary!.summaries[0].summary.approximate,[]);
    assert.equal(fixture.requests.length,19,'state, summary and the reached event page each reread once when accepted coverage advances');
    assert.equal(native.events!.coverage!.complete,true);
    assert.equal(statuses,3);
  } finally {for(const slot of fixture.slots)slot.cleanup?.();await vite.close();delete host.__partyHook;delete host.__partyChart;}
});
