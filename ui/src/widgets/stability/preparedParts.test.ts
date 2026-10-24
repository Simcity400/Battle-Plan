import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { drawBands } from '../../charts/draw.ts';
import { plotFrame } from '../../charts/frame.ts';
import { partLayers, partsStack, partsAt } from './partsData.ts';
import type { MeasureParts } from '../../generated/dashboard';
import type { ChartTokens } from '../../charts/tokens';
import type { MeasureSummaries, MeasureStates } from '../country-measure/preparedMeasureData';
import type * as Helpers from './preparedParts';

test('prepared exact part runs preserve palette, raw sum order, gaps, and legacy signed band paths', async () => {
  const vite = await createServer({configFile:false,server:{middlewareMode:true,watch:null,hmr:false,ws:false}});
  try {
    const helpers = await vite.ssrLoadModule('/ui/src/widgets/stability/preparedParts.ts') as typeof Helpers;
    const observed = [{firstHour:0,lastHour:4},{firstHour:6,lastHour:8}];
    const parts:MeasureParts = {observed,parts:[
      {id:'term:base',term:'base',line:null,runs:[{firstHour:0,lastHour:4,valueRaw:'9223372036854775807',startsSegment:true},{firstHour:6,lastHour:8,valueRaw:'53',startsSegment:true}]},
      {id:'remainder',term:null,line:null,runs:[{firstHour:0,lastHour:1,valueRaw:'-9223372036854775806',startsSegment:true},{firstHour:2,lastHour:4,valueRaw:'-50001',startsSegment:false}]},
      {id:'clamp',term:null,line:null,runs:[{firstHour:6,lastHour:8,valueRaw:'-800000',startsSegment:true}]},
    ]};
    const series = ['',...parts.parts.map(part=>part.id)].map(id=> {
      const runs = id ? parts.parts.find(part=>part.id===id)!.runs : observed.map(span=>({...span,valueRaw:'0',startsSegment:true}));
      return {tag:JSON.stringify(['country / 5000',id]),marks:null,summary:{from:0,to:8,width:100,
        runs:runs.map(run=>({firstHour:run.firstHour,lastHour:run.lastHour,value:Number(run.valueRaw)/100000,breakBefore:run.startsSegment})),
        buckets:Array.from({length:9},(_,hour)=> {const run=runs.find(run=>run.firstHour<=hour && run.lastHour>=hour);if(!run)return null;const value=Number(run.valueRaw)/100000;const raw=BigInt(run.valueRaw);return {firstHour:hour,lastHour:hour,firstValue:value,lastValue:value,lastValueHour:run.firstHour,minHour:hour,maxHour:hour,minValue:value,maxValue:value,firstBreak:run.startsSegment && hour===run.firstHour,hasBreak:false,absoluteTotal:String(raw<0n ? -raw : raw)};})}};
    });
    const data = {branchId:'1',measure:'stability',decimals:5,series,warnings:[],parts:{'country / 5000':{...parts,parts:parts.parts.map(part=>({...part,runs:[]})),observed:[]}}} as MeasureSummaries;
    const geometry = helpers.preparedPartsGeometry(data);
    assert.deepEqual(geometry,parts);
    const weights=helpers.preparedPartsWeights(data);
    const layers=partLayers(parts,'stability',['#123','#456'],'#888');
    assert.deepEqual(partLayers(geometry,'stability',['#123','#456'],'#888',weights),layers);
    const legacy=partsStack(parts,layers,5,0,8),prepared=partsStack(geometry,layers,5,0,8);
    assert.deepEqual(prepared,legacy);
    const frame=plotFrame(400,240,{left:30,right:10,top:10,bottom:30},[0,8],[-1e16,1e16],false);
    const tokens={lineWidth:{series:2}} as ChartTokens;
    const path=(segments:typeof legacy)=> {const calls:unknown[]=[];const ctx=new Proxy({},{get:(_target,name)=> (...args:unknown[])=>calls.push([name,...args]),set:()=>true}) as CanvasRenderingContext2D;
      for(const segment of segments) drawBands(ctx,frame,tokens,segment.hours,segment.bands,segment.layers.map(layer=>layers[layer].color)); return calls;};
    assert.deepEqual(path(prepared),path(legacy));
  } finally {await vite.close();}
});

test('dense buckets rank the whole range with exact contribution totals and keep breaks',async()=> {
  const vite=await createServer({configFile:false,server:{middlewareMode:true,watch:null,hmr:false,ws:false}});
  try {const {preparedPartsWeights,preparedPartsGeometry,preparedPartsAt}=await vite.ssrLoadModule('/ui/src/widgets/stability/preparedParts.ts') as typeof Helpers;
    const names:MeasureParts={parts:['a','b'].map(id=>({id,term:'base',line:null,runs:[]})),observed:[]};
    const bucket={firstHour:0,lastHour:1_000_000,firstValue:0,lastValue:0,lastValueHour:1_000_000,minHour:5,minValue:-2,maxHour:6,maxValue:3,firstBreak:true,hasBreak:false};
    const data={branchId:'1',measure:'stability',decimals:5,warnings:[],parts:{tag:names},series:['','a','b'].map(id=>({tag:JSON.stringify(['tag',id]),marks:null,summary:{from:0,to:1_000_000,width:1,buckets:[{...bucket,absoluteTotal:id==='b' ? '92233720368547758080000' : '92233720368547758070000'}]}}))} as MeasureSummaries;
    const geometry=preparedPartsGeometry(data),weights=preparedPartsWeights(data);
    assert.equal(weights.get('b'),92233720368547758080000n);
    assert.equal(partLayers(geometry,'stability',['#123'],'#888',weights)[0].id,'b');
    assert.ok(partsStack(geometry,partLayers(geometry,'stability',['#123'],'#888',weights),5,0,1_000_000)[0].hours.length<20);
    data.series[2].summary.approximate=[{firstHour:5,lastHour:999_999}];
    assert.deepEqual(preparedPartsGeometry(data).approximate,[{firstHour:5,lastHour:999_999}],'the signed stack carries shared approximate spans');
    data.series[0].summary.buckets[0]!.hasBreak=true;
    assert.deepEqual(preparedPartsGeometry(data).observed.map(({firstHour,lastHour})=>({firstHour,lastHour})),[{firstHour:0,lastHour:0},{firstHour:1_000_000,lastHour:1_000_000}]);
    const states={branchId:'1',measure:'stability',decimals:5,warnings:[],parts:{tag:names,reused:names},states:[
      {tag:JSON.stringify(['tag','']),hour:9,run:{firstHour:8,lastHour:10,valueRaw:'8',startsSegment:true,slot:1,countryId:1},drop:null},
      {tag:JSON.stringify(['tag','a']),hour:9,run:{firstHour:8,lastHour:10,valueRaw:'9223372036854775807',startsSegment:true,slot:1,countryId:1},drop:null},
      {tag:JSON.stringify(['reused','']),hour:9,run:{firstHour:8,lastHour:10,valueRaw:'8',startsSegment:true,slot:2,countryId:2},drop:null},
      {tag:JSON.stringify(['reused','b']),hour:9,run:{firstHour:8,lastHour:10,valueRaw:'77',startsSegment:true,slot:2,countryId:2},drop:null},
    ]} as MeasureStates;
    const within=new Map([['tag',[{firstHour:0,lastHour:10}]],['reused',[{firstHour:11,lastHour:20}]]]);
    assert.deepEqual(partsAt(preparedPartsAt(states,9,within),9)?.map(({part,raw})=>[part.id,raw]),[['a','9223372036854775807']]);
    assert.equal(partsAt(preparedPartsAt(states,5,within),5),null);
    const estimated={...states,states:states.states.map(state=>({...state,approximate:true}))};
    assert.equal(partsAt(preparedPartsAt(estimated,9,within),9),null,'block estimates never become exact hover or parity values');
    const pending={...states,states:states.states.map(state=>state.tag===JSON.stringify(['tag','']) ? {...state,pending:true} : state)};
    assert.deepEqual(partsAt(preparedPartsAt(pending,9,within),9)?.map(({part,raw})=>[part.id,raw]),[['a','9223372036854775807']],'pending character parts keep independently known formula values');
    const first={parts:[{id:'z',term:'base',line:null,runs:[]}],observed:[]};
    const last={parts:[{id:'a',term:'war',line:null,runs:[]},{id:'z',term:'war',line:null,runs:[]}],observed:[]};
    const renamed={...states,parts:{first,last},states:['','a','z'].map(id=>({...states.states[0],tag:JSON.stringify(['last',id])}))} as MeasureStates;
    const projected=preparedPartsAt(renamed,9,new Map([['first',[]],['last',[{firstHour:6,lastHour:10}]]]),['first','last']);
    assert.deepEqual(projected.parts.map(part=>[part.id,part.term]),[['z','base'],['a','war']],'aliases keep the legacy inventory order and first naming row');
    assert.throws(()=>preparedPartsWeights({...data,series:data.series.map(line=>({...line,summary:{...line.summary,buckets:[bucket]}}))}),/contribution/);
  } finally {await vite.close();}
});

test('a tail-only modifier and an unavailable character do not erase known dense formula bands',async()=> {
  const vite=await createServer({configFile:false,server:{middlewareMode:true,watch:null,hmr:false,ws:false}});
  try {
    const {preparedPartsGeometry}=await vite.ssrLoadModule('/ui/src/widgets/stability/preparedParts.ts') as typeof Helpers;
    const bucket=(firstHour:number,lastHour:number,value:number)=>({firstHour,lastHour,firstValue:value,lastValue:value,lastValueHour:firstHour,minHour:firstHour,maxHour:firstHour,minValue:value,maxValue:value,firstBreak:true,hasBreak:false,absoluteTotal:String(Math.abs(value)*100000*(lastHour-firstHour+1))});
    const data={branchId:'1',measure:'stability',decimals:5,warnings:[],parts:{tag:{observed:[],parts:[
      {id:'term:base',term:'base',line:null,runs:[]},
      {id:'line:idea:mod/tail:0:0',term:null,line:null,runs:[]},
      {id:'line:countryLeader:7',term:null,line:null,runs:[]},
    ]}},series:[
      {tag:JSON.stringify(['tag','']),marks:null,summary:{from:0,to:9,width:2,buckets:[bucket(0,4,0),bucket(5,9,0)]}},
      {tag:JSON.stringify(['tag','term:base']),marks:null,summary:{from:0,to:9,width:2,buckets:[bucket(0,4,.5),bucket(5,9,.5)]}},
      {tag:JSON.stringify(['tag','line:idea:mod/tail:0:0']),marks:null,summary:{from:0,to:9,width:2,buckets:[null,bucket(5,9,.1)],approximate:[{firstHour:5,lastHour:9}]}},
      {tag:JSON.stringify(['tag','line:countryLeader:7']),marks:null,summary:{from:0,to:9,width:2,buckets:[null,null]}},
    ]} as MeasureSummaries;
    const geometry=preparedPartsGeometry(data);
    assert.deepEqual(geometry.approximate,[{firstHour:5,lastHour:9}]);
    assert.deepEqual(partsAt(geometry,0)?.map(({part,raw})=>[part.id,raw]),[['term:base','50000']],'a missing sparse bucket never removes the known base band');
    assert.deepEqual(partsAt(geometry,9)?.map(({part,raw})=>[part.id,raw]),[['term:base','50000'],['line:idea:mod/tail:0:0','10000']]);
    const bands=partsStack(geometry,partLayers(geometry,'stability',['#123','#456'],'#888'),5,0,9);
    assert.ok(bands.length && bands[0].bands.length,'the known contributions draw before and after the modifier appears');
    assert.ok(bands.flatMap(segment=>segment.hours).length<=12,'dense geometry stays bounded by the screen columns and parts');
    assert.equal(partsAt(geometry,10),null,'outside observed coverage remains unknown');
  } finally {await vite.close();}
});

test('the drill uses bounded summaries and exact hours, and both paths register the part parity model',async()=> {
  const {createElement}=await import('react'),{renderToStaticMarkup}=await import('react-dom/server');
  type Slot={value?:unknown;deps?:readonly unknown[];cleanup?:()=>void};
  const fixture={cursor:0,slots:[] as Slot[],effects:[] as (()=>void)[],chart:undefined as import('../../charts/TimeChart').TimeChartProps|undefined,revision:0,pending:false,queries:[] as {product:string;params:Record<string,unknown>}[],model:undefined as (()=>import('../../dashboard/parityRows').ParityList)|undefined};
  const host=globalThis as typeof globalThis & {__partsFixture?:typeof fixture;__partsHook?:(name:string,...args:unknown[])=>unknown};host.__partsFixture=fixture;
  host.__partsHook=(name,...args)=> {
    const slot=fixture.slots[fixture.cursor++] ??= {};
    if(name==='useMemo'||name==='useCallback'||name==='useEffect') {const [factory,deps]=args as [()=>unknown,readonly unknown[]];
      if(!slot.deps||slot.deps.length!==deps.length||deps.some((value,index)=>!Object.is(value,slot.deps![index]))) {slot.deps=[...deps];if(name==='useMemo')slot.value=factory();else if(name==='useCallback')slot.value=factory;else fixture.effects.push(()=>{slot.cleanup?.();const result=factory();slot.cleanup=typeof result==='function' ? result as ()=>void : undefined;});}return slot.value;
    }
    if(!('value' in slot)) slot.value=name==='useRef' ? {current:args[0]} : args[0];
    if(name==='useRef')return slot.value;
    return [slot.value,(next:unknown)=>{slot.value=typeof next==='function' ? (next as (previous:unknown)=>unknown)(slot.value) : next;}];
  };
  const replacements:Record<string,string>={
    '/charts/TimeChart.tsx':'export default props => {globalThis.__partsFixture.chart=props;return null;};',
    '/charts/useChartTokens.ts':'export const useViewportHeight=()=>1000;',
    '/dashboard/contentRevision.ts':'export const useContentRevision=()=>globalThis.__partsFixture.revision;',
    '/dashboard/parityPreparation.ts':'export const useParityPreparation=()=>({requested:true,prepare(){}});',
    '/dashboard/parityRows.ts':'export const useParityRows=(_root,model)=>{globalThis.__partsFixture.model=model;};',
  };
  const vite=await createServer({mode:'parity',configFile:false,optimizeDeps:{noDiscovery:true},server:{middlewareMode:true,watch:null,hmr:false,ws:false},plugins:[{name:'parts-drill-fixture',enforce:'pre',transform(source,id){const path=id.replaceAll('\\','/');const replacement=Object.entries(replacements).find(([suffix])=>path.endsWith(suffix));if(replacement)return replacement[1];if(['/widgets/stability/StabilityDrill.tsx','/widgets/stability/preparedParts.ts','/widgets/country-measure/preparedMeasure.ts','/dashboard/viewRead.ts'].some(suffix=>path.endsWith(suffix)))return source.replace(/\b(useMemo|useState|useRef|useEffect|useCallback)(?:<[^>]+>+)?\(/g,(_match,name)=>`globalThis.__partsHook(${JSON.stringify(name)}, `);}}]});
  try {
    const {default:Drill}=await vite.ssrLoadModule('/ui/src/widgets/stability/StabilityDrill.tsx');
    const tag='country / 5000',raw='9223372036854775807';
    const parts:MeasureParts={observed:[{firstHour:0,lastHour:10}],parts:[{id:'term:base',term:'base',line:null,runs:[{firstHour:0,lastHour:10,valueRaw:raw,startsSegment:true}]}]};
    const client={preparedQuery:async (_source:unknown,product:string,params:Record<string,unknown>,options:{signal:AbortSignal})=> {
      fixture.queries.push({product,params});assert.equal(options.signal.aborted,false);
      assert.ok(fixture.queries.length<=24,'mounted views issue a bounded number of requests');
      const metadata={parts:{[tag]:{...parts,parts:parts.parts.map(part=>({...part,runs:[]})),observed:[]}}};
      if(product==='series-state')return {branchId:'1',measure:'stability',decimals:5,warnings:[],...metadata,states:(params.reads as {tag:string;hour:number}[]).flatMap(read=>['','term:base'].map(id=>({tag:JSON.stringify([read.tag,id]),hour:read.hour,pending:!id && fixture.pending,run:read.hour<=10 ? {firstHour:0,lastHour:10,valueRaw:id ? raw : '0',slot:1,countryId:1,startsSegment:true,readHour:Math.min(read.hour,8)} : null,drop:null})))};
      if(product==='series-next-change')return {hour:8};
      assert.equal(product,'series-summary');const from=Number(params.from),to=Number(params.to),width=Number(params.width);
      return {branchId:'1',measure:'stability',decimals:5,warnings:[],...metadata,series:['','term:base'].map(id=>({tag:JSON.stringify([tag,id]),marks:null,summary:{from,to,width,
        approximate:[{firstHour:5,lastHour:10}],
        ...(width>1 ? {runs:[{firstHour:0,lastHour:10,value:Number(id ? raw : '0')/100000,breakBefore:true}]} : {}),
        buckets:Array.from({length:width>1 ? to-from+1 : 1},(_,index)=>{const hour=from+index;const value=Number(id ? raw : '0')/100000;return {firstHour:hour,lastHour:width>1 ? hour : to,firstValue:value,lastValue:value,lastValueHour:hour,minHour:hour,maxHour:hour,minValue:value,maxValue:value,firstBreak:hour===0,hasBreak:false,absoluteTotal:String(BigInt(id ? raw : '0')*BigInt(width>1 ? 1 : to-from+1))};})}}))};
    }};
    const props={reply:{branchId:'1',tag,decimals:5,stability:parts,warSupport:parts,warnings:[]},numbers:{branchId:'1',tag,decimals:5,stability:parts,warSupport:parts,warnings:[]},pickOwner:{},presentationReady:true,measure:'stability',hour:9,extent:[0,10],playback:{playhead:null,mode:'draw-in',afterPlayhead:false,windowHours:24},branchStart:0,hoursPerSecond:24,active:true,tokens:{categories:['#123'],muted:'#888'},within:null,readAt:()=>8,
      preparedBinding:{client,source:{id:'source',revision:'1'},branchId:'1',tags:[tag]}};
    const render=()=>{fixture.cursor=0;fixture.effects=[];const html=renderToStaticMarkup(createElement(Drill,props));for(const effect of fixture.effects)effect();return html;};
    const settle=async()=>{for(let i=0;i<6;i++){await new Promise(resolve=>setImmediate(resolve));render();}};
    render();fixture.chart!.onSummaryView!({from:0,to:10,width:100});render();await settle();
    assert.ok(fixture.queries.some(query=>query.product==='series-summary' && query.params.width===1));
    assert.ok(fixture.queries.some(query=>query.product==='series-summary' && query.params.width===100));
    assert.equal(fixture.model!().label,'Stability part values');assert.equal(fixture.model!().complete,true);
    const preparedModel=fixture.model!().rows;
    assert.ok(render().includes('As of'));
    const approximateMarkup=render();assert.ok(approximateMarkup.includes('Approximate'),approximateMarkup);
    const paints:{alpha:number;dash:number[]}[]=[],saved:{alpha:number;dash:number[]}[]=[];
    let alpha=1,dash:number[]=[];
    const ctx={save(){saved.push({alpha,dash});},restore(){({alpha,dash}=saved.pop()!);},get globalAlpha(){return alpha;},set globalAlpha(value:number){alpha=value;},setLineDash(value:number[]){dash=value;},beginPath(){},rect(){},clip(){},moveTo(){},lineTo(){},closePath(){},fill(){paints.push({alpha,dash});},stroke(){}} as unknown as CanvasRenderingContext2D;
    const frame=plotFrame(140,140,{left:20,right:20,top:20,bottom:20},[0,10],[-1e16,1e16],false);
    fixture.chart!.layers[0].draw(ctx,frame,props.tokens as unknown as ChartTokens,{playhead:null});
    assert.ok(paints.some(paint=>paint.alpha===1 && paint.dash.length===0));
    assert.ok(paints.some(paint=>paint.alpha===.55 && paint.dash.join(',')==='4,4'),'stability bands use the shared approximate renderer');
    assert.equal(alpha,1);assert.deepEqual(dash,[]);
    assert.ok(fixture.chart!.tooltip!(9,undefined as never,[])!.rows!.some(row=>row.label==='Total'));
    fixture.pending=true;fixture.chart!.onHover!(6);render();await settle();
    assert.ok(render().includes('Still being prepared...'));
    assert.ok(fixture.chart!.tooltip!(9,undefined as never,[])!.rows!.some(row=>row.label==='Base value'),'known formula rows remain exact while a character part is pending');
    assert.equal(fixture.chart!.tooltip!(9,undefined as never,[])!.rows!.find(row=>row.label==='Total')!.value,'Still being prepared...');
    assert.ok(!render().includes('>100%</small>'),'unknown totals never supply shares');
    fixture.pending=false;
    fixture.chart!.onHover!(5);render();await settle();assert.ok(fixture.queries.some(query=>query.product==='series-state' && (query.params.reads as {hour:number}[]).some(read=>read.hour===5)));
    fixture.chart!.onNavigateHour!('ArrowRight',4,[0,10],null);await settle();assert.equal(fixture.chart!.keyboardHour,8);
    const count=fixture.queries.length;fixture.revision++;render();await settle();assert.ok(fixture.queries.length>count,'content changes refresh naming without discarding the numeric owner');
    for(const slot of fixture.slots)slot.cleanup?.();fixture.slots=[];delete (props.preparedBinding.client as {preparedQuery?:unknown}).preparedQuery;
    render();await settle();assert.deepEqual(fixture.model!().rows,preparedModel,'legacy and prepared parity values use the same canonical model');
  } finally {for(const slot of fixture.slots)slot.cleanup?.();await vite.close();delete host.__partsFixture;delete host.__partsHook;}
});
