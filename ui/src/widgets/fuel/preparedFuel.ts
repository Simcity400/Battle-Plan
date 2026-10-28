import type { RefObject } from 'react';
import { useParityRows } from '../../dashboard/parityRows';
import type { ParityList } from '../../dashboard/parityRows';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { isMember } from '../../dashboard/groups';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { FuelRead, FuelReply, FuelConsumptionRead, FuelConsumptionReply, HourSpan } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import type { Run } from '../../charts/series';
import type { TimeChartProps } from '../../charts/TimeChart';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useViewRead } from '../../dashboard/viewRead';
import { object, signed32 } from '../../dashboard/validation';
import { validateFuelReply } from './fuelData';
import { validateFuelConsumption } from './fuelConsumption';
import { validateMeasureSummaries } from '../country-measure/preparedMeasureData';

export interface FuelPoint { tag: string; hour: number; recorded: boolean; fuel: FuelRead | null; consumption: FuelConsumptionRead | null; consumptionSegment: HourSpan | null; fuelRaw?: string; capacityRaw?: string; approximate?: boolean }
export interface FuelStates { branchId: string; states: FuelPoint[]; replies: FuelReply[] }
export interface FuelSummaries { branchId: string; series: { tag: string; measure: string; summary: ChartSummary }[]; replies: FuelReply[] }
export function validateFuelStates(value: unknown): FuelStates {
  if (!object(value) || typeof value.branchId !== 'string' || !Array.isArray(value.states) || !Array.isArray(value.replies)) throw new Error('Prepared fuel state is malformed.');
  const result = value as unknown as FuelStates;
  result.replies.forEach(validateFuelReply);
  const metadataByTag = new Map(result.replies.map(reply => [reply.tag, reply]));
  for (const state of result.states) {
    if (!object(state) || typeof state.tag !== 'string' || !signed32(state.hour) || typeof state.recorded !== 'boolean' || state.fuel !== null && !object(state.fuel) || state.consumption !== null && !object(state.consumption)) throw new Error('Prepared fuel point is malformed.');
    const metadata = metadataByTag.get(state.tag);
    if (!metadata) throw new Error('Prepared fuel metadata is missing.');
    for (const amount of [state.fuelRaw,state.capacityRaw]) if (amount!==undefined && (typeof amount!=='string' || !/^-?\d+$/.test(amount))) throw new Error('Prepared fuel amount is malformed.');
    validateFuelReply({ ...metadata, reads: state.fuel ? [state.fuel] : [] });
    validateFuelConsumption({ branchId: result.branchId, tag: state.tag, reads: state.consumption ? [state.consumption] : [], warnings: [] });
    if (state.fuel && (state.fuel.firstHour > state.hour || state.fuel.lastHour < state.hour)
      || state.consumption && (state.consumption.firstHour > state.hour || state.consumption.lastHour < state.hour)) throw new Error('Prepared fuel point does not cover its hour.');
  }
  return result;
}
export function validateFuelSummaries(value: unknown): FuelSummaries {
  if (!object(value) || typeof value.branchId !== 'string' || !Array.isArray(value.series) || !Array.isArray(value.replies)) throw new Error('Prepared fuel summaries are malformed.');
  const result = value as unknown as FuelSummaries;
  result.replies.forEach(validateFuelReply);
  validateMeasureSummaries({ ...result, measure: 'politicalPower', decimals: 5, warnings: [], series: result.series.map(line=>({...line,marks:null})) });
  if (result.series.some(line => typeof line.measure !== 'string')) throw new Error('Prepared fuel measure is malformed.');
  return result;
}
function reachedReads<R extends { firstHour: number }>(states: FuelStates, select: (point: FuelPoint) => R | null) {
  const countries = new Map<string, Map<number, R>>();
  for (const point of states.states) {
    const read = select(point);
    if (!read) continue;
    let reads = countries.get(point.tag);
    if (!reads) { reads = new Map(); countries.set(point.tag, reads); }
    reads.set(read.firstHour, read);
  }
  return new Map([...countries].map(([tag, reads]) => [tag, [...reads.values()].sort((a,b) => a.firstHour-b.firstHour)]));
}
export function reachedFuel(states: FuelStates | undefined): FuelReply[] | undefined {
  if (!states) return undefined;
  const reads = reachedReads(states, point => point.fuel);
  return states.replies.map(reply => ({ ...reply, reads: reads.get(reply.tag) ?? [] }));
}
export function reachedConsumption(states: FuelStates | undefined): FuelConsumptionReply[] | undefined {
  if (!states) return undefined;
  const reads = reachedReads(states, point => point.consumption);
  return states.replies.map(reply => ({ branchId: reply.branchId, tag: reply.tag, warnings: [], reads: reads.get(reply.tag) ?? [] }));
}
/** Each country's chart endpoints add reads only for that country. */
export function fuelReadRequests(tags: readonly string[], hours: readonly number[], summaries: FuelSummaries | undefined) {
  const wanted = new Map(tags.map(tag => [tag, new Set(hours)]));
  for (const line of summaries?.series ?? []) {
    for (let index = line.summary.buckets.length - 1; index >= 0; index--) {
      const bucket = line.summary.buckets[index];
      if (bucket) { wanted.get(line.tag)?.add(bucket.lastHour); break; }
    }
  }
  return [...wanted].flatMap(([tag, hours]) => [...hours].sort((a,b) => a-b).map(hour => ({tag,hour})));
}
/** Missing requested dates cannot advance a state or parity page. */
export function fuelStateIndex(data: FuelStates, requested: readonly { tag: string; hour: number }[]) {
  const countries = new Map<string, Map<number, FuelPoint>>();
  for (const point of data.states) {
    let hours = countries.get(point.tag);
    if (!hours) { hours = new Map(); countries.set(point.tag, hours); }
    if (hours.has(point.hour)) throw new Error('Prepared fuel reply repeats an exact hour.');
    hours.set(point.hour, point);
  }
  if (requested.some(read => !countries.get(read.tag)?.has(read.hour))) throw new Error('Prepared fuel reply did not advance the requested hours.');
  return countries;
}
export function scaledFuelSummary(summary: ChartSummary | undefined, scale: number): ChartSummary | undefined {
  return summary && { ...summary, runs: summary.runs?.map(run => ({ ...run, value: run.value / scale })),
    buckets: summary.buckets.map(bucket => bucket && ({ ...bucket, firstValue: bucket.firstValue / scale, lastValue: bucket.lastValue / scale,
      minValue: bucket.minValue / scale, maxValue: bucket.maxValue / scale })) };
}
/** Exact windows retain supplying runs; dense stacks use the shared bucket extrema. */
export function fuelSummaryRuns(summary: ChartSummary | undefined, scale: number): Run[] {
  const scaled = scaledFuelSummary(summary, scale);
  if (!scaled) return [];
  if (scaled.runs) return [...scaled.runs];
  return scaled.buckets.flatMap(bucket => {
    if (!bucket) return [];
    const ordered = [...new Map([[bucket.firstHour,bucket.firstValue],[bucket.minHour,bucket.minValue],
      [bucket.maxHour,bucket.maxValue],[bucket.lastValueHour,bucket.lastValue],[bucket.lastHour,bucket.lastValue]])].sort((a,b)=>a[0]-b[0]);
    return ordered.map(([hour,value],index)=>({firstHour:hour,lastHour:bucket.hasBreak ? hour : ordered[index+1] ? ordered[index+1][0]-1 : bucket.lastHour,
      value,breakBefore:bucket.hasBreak || index===0 && bucket.firstBreak}));
  });
}

export function usePreparedFuel(client: DashboardClient, track: BoundTrack | undefined, tags: readonly string[], hour: number,
  active: boolean, measures: readonly string[], within?: ReadonlyMap<string,readonly HourSpan[]>, originHour?: number) {
  const enabled = !!client.preparedQuery && !!track;
  const content = useContentRevision(client);
  const identity = JSON.stringify([track?.source.id,track?.source.revision,track?.branch.id,tags,measures,within && [...within]]);
  const owner = useMemo(()=>({client,identity}),[client,identity]);
  const alive = useRef(owner);alive.current=owner;
  const [view,setView]=useState<{owner:typeof owner;from:number;to:number;width:number}>();
  const [hover,setHover]=useState<number|null>(null);
  const [states,setStates]=useState<{owner:typeof owner;data:FuelStates}>();
  const [error,setError]=useState<{owner:typeof owner;message:string;reads?:string}>();
  const [attempt,setAttempt]=useState(0);
  const [keyboard,setKeyboard]=useState<{owner:typeof owner;hour:number|null;revision:number}>();
  const requests=useRef(new Set<AbortController>());
  const onSummaryView=useCallback<NonNullable<TimeChartProps['onSummaryView']>>(value=>setView(previous=>previous?.owner===owner
    && previous.from===value.from && previous.to===value.to && previous.width===value.width ? previous : {owner,...value}),[owner]);
  const currentView=view?.owner===owner ? view : undefined;
  // One summary read at a time while the chart's view moves (`useViewRead`).
  const summaries=useViewRead<{from:number;to:number;width:number},FuelSummaries>(enabled ? JSON.stringify([identity,content]) : null,currentView,
    JSON.stringify([currentView?.from,currentView?.to,currentView?.width]),active,(at,signal,publish)=>preparedSeriesQuery(client,track!.source,'series-summary',
      {branchId:track!.branch.id,family:{kind:'country-fuel'},tags,measures,from:at.from,to:at.to,width:at.width,within:within && Object.fromEntries(within)},
      signal,attempt>0,undefined,undefined,value=>publish(validateFuelSummaries(value))).then(validateFuelSummaries), client);
  const currentSummary=summaries.data;
  const wanted=[Math.floor(hour),...(originHour===undefined?[]:[originHour]),...(hover===null?[]:[Math.floor(hover)]),...(currentView?[Math.floor(currentView.from),Math.floor(currentView.to)]:[])];
  const reads=fuelReadRequests(tags,wanted,currentSummary);
  const readKey=JSON.stringify(reads);
  // One state read at a time while playback and the hover move (`useViewRead`): the last
  // states stay shown, and a slow read lands instead of being cancelled every hour.
  // A shown partial reply lets a moved view replace a read still preparing.
  useViewRead<typeof reads,null>(enabled ? JSON.stringify([identity,content,attempt]) : null,reads,readKey,active,(at,signal,publish)=>
    preparedSeriesQuery(client,track!.source,'series-state',{branchId:track!.branch.id,family:{kind:'country-fuel'},reads:at},signal,attempt>0,undefined,undefined,
      value=>{if(!signal.aborted && alive.current===owner){const data=validateFuelStates(value);fuelStateIndex(data,at);setStates({owner,data});publish(null);}})
      .then(()=>null,reason=>{if(!signal.aborted && alive.current===owner)setError({owner,message:String(reason),reads:JSON.stringify(at)});return null;}),client);
  useEffect(()=>()=>{for(const controller of requests.current)controller.abort();requests.current.clear();},[owner,active]);
  const exactAt=useCallback(async(at:number)=>{
    if(!enabled || !active)return null;
    const controller=new AbortController();requests.current.add(controller);
    try {
      const reads=fuelReadRequests(tags,[Math.floor(at)],undefined);
      const data=validateFuelStates(await preparedSeriesQuery(client,track!.source,'series-state',{branchId:track!.branch.id,family:{kind:'country-fuel'},reads},controller.signal,attempt>0));
      if(controller.signal.aborted || alive.current!==owner)return null;
      fuelStateIndex(data,reads);
      setHover(Math.floor(at));setStates({owner,data});return data;
    }catch(reason){if(!controller.signal.aborted && alive.current===owner)setError({owner,message:String(reason)});return null;}
    finally{requests.current.delete(controller);}
  },[owner,enabled,active,attempt,content]);
  const onNavigateHour=useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((key,from,x,until)=>{
    if(!enabled || !active)return;
    const controller=new AbortController();requests.current.add(controller);
    void preparedSeriesQuery(client,track!.source,'series-next-change',{branchId:track!.branch.id,family:{kind:'country-fuel'},tags,
      key,from,firstHour:x[0],lastHour:Math.min(x[1],until??x[1]),within:within && Object.fromEntries(within)},controller.signal,attempt>0)
      .then(value=>{if(!object(value) || value.hour!==null && (typeof value.hour!=='number' || !Number.isFinite(value.hour)))throw new Error('Prepared fuel navigation is malformed.');
        if(!controller.signal.aborted && alive.current===owner)setKeyboard(previous=>({owner,hour:value.hour as number|null,revision:(previous?.revision??0)+1}));})
      .catch(reason=>{if(!controller.signal.aborted && alive.current===owner)setError({owner,message:String(reason)});}).finally(()=>requests.current.delete(controller));
  },[owner,enabled,active,attempt]);
  const current=states?.owner===owner ? states.data : undefined;
  const data=useMemo(()=>reachedFuel(current) ?? currentSummary?.replies,[current,currentSummary]);
  const consumption=useMemo(()=>current?.states.some(point=>point.fuelRaw!==undefined) ? undefined : reachedConsumption(current),[current]);
  return {enabled,owner,data,consumption,states:current,summaries:currentSummary,
    amountsAt:(tag:string,hour:number)=>{const point=current?.states.find(point=>point.tag===tag && point.hour===Math.floor(hour));
      return !point?.approximate && point?.fuelRaw!==undefined && point.capacityRaw!==undefined ? {fuelRaw:point.fuelRaw,capacityRaw:point.capacityRaw} : undefined;},
    summary:(tag:string,measure:string)=>currentSummary?.series.find(line=>line.tag===tag && line.measure===measure)?.summary,
    onSummaryView,onHover:setHover,onNavigateHour,keyboardHour:keyboard?.owner===owner?keyboard.hour:undefined,
    keyboardRevision:keyboard?.owner===owner?keyboard.revision:undefined,exactAt,
    // A state read's failure belongs to the reads it asked for; a newer set is still loading.
    error:(error?.owner===owner && (error.reads===undefined || error.reads===readKey)?error.message:undefined)
      ?? (summaries.errorKey===JSON.stringify([currentView?.from,currentView?.to,currentView?.width]) ? summaries.error : undefined),retry:()=>{setError(undefined);setAttempt(value=>value+1);summaries.retry();}};
}

/** Both strip halves keep their own dated peak; exact bins retain every day. */
export function fuelSummaryStrip(gain: ChartSummary | undefined, burn: ChartSummary | undefined, reply: Pick<FuelReply,'dayHours'|'gainScale'|'quantityScale'>) {
  const bars = new Map<number,{firstHour:number;lastHour:number;up:number;down:number}>();
  for (const [summary,field,scale] of [[gain,'up',reply.gainScale],[burn,'down',reply.quantityScale]] as const) {
    const points = summary?.runs?.map(run=>[run.firstHour,run.value] as const)
      ?? summary?.buckets.flatMap(bucket=>bucket?[[bucket.maxHour,bucket.maxValue] as const]:[]) ?? [];
    for (const [hour,value] of points) {
      const bar=bars.get(hour) ?? {firstHour:hour-reply.dayHours,lastHour:hour,up:0,down:0};
      bar[field]=value/scale;bars.set(hour,bar);
    }
  }
  return [...bars.values()].sort((a,b)=>a.lastHour-b.lastHour);
}

/** Full hourly values are collected only for the browser parity handshake. */
export function useFuelSeriesParity(root: RefObject<Element | null>, client: DashboardClient, track: BoundTrack | undefined,
  tags: readonly string[], extent: readonly [number,number], within: ReadonlyMap<string,readonly HourSpan[]> | undefined,
  label: string, legacy: readonly FuelReply[] | readonly FuelConsumptionReply[] | undefined, consumption = false) {
  const {requested,prepare}=useParityPreparation();
  const key=JSON.stringify([track?.source.id,track?.source.revision,track?.branch.id,tags,extent,within && [...within],label,consumption]);
  const [answer,setAnswer]=useState<{key:string;client:DashboardClient;model?:ParityList;error?:string}>();
  useEffect(()=>{
    if(!requested || !track) return;
    const controller=new AbortController();
    void (async()=>{
      const rows:unknown[]=[];
      for(let first=Math.floor(extent[0]);first<=extent[1];first+=128){
        const hours=Array.from({length:Math.min(128,Math.floor(extent[1])-first+1)},(_,index)=>first+index);
        const wanted=tags.flatMap(tag=>hours.filter(hour=>isMember(within?.get(tag),hour)).map(hour=>({tag,hour})));
        if(!wanted.length) continue;
        const data=client.preparedQuery ? validateFuelStates(await preparedSeriesQuery(client,track.source,'series-state',
          {branchId:track.branch.id,family:{kind:'country-fuel'},reads:wanted},controller.signal,true)) : undefined;
        if(controller.signal.aborted) return;
        const points=data ? fuelStateIndex(data,wanted) : undefined;
        for(const {tag,hour} of wanted){
          const point=points?.get(tag)?.get(hour);
          const fallback=legacy?.find(reply=>reply.tag===tag)?.reads.find(read=>read.firstHour<=hour && read.lastHour>=hour);
          rows.push({tag,hour,read:data ? consumption ? point?.consumption ?? null : point?.fuel ?? null : fallback ?? null});
        }
      }
      if(!controller.signal.aborted)setAnswer({key,client,model:{kind:'chart',label,complete:true,total:rows.length,rows}});
    })().catch(reason=>{if(!controller.signal.aborted)setAnswer({key,client,error:String(reason)});});
    return ()=>controller.abort();
  },[requested,key,client,legacy]);
  const current=answer?.key===key && answer.client===client ? answer : undefined;
  useParityRows(root,()=>current?.model ?? {kind:'chart',label,complete:false,total:0,rows:[]},{prepare,ready:()=>!!current?.model,error:()=>current?.error??null});
}
