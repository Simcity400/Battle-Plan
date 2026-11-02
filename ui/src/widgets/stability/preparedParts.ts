import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { HourSpan, MeasureParts } from '../../generated/dashboard';
import { useContentRevision } from '../../dashboard/contentRevision';
import { usePreparedMeasure } from '../country-measure/preparedMeasure';
import type { MeasureStates, MeasureSummaries } from '../country-measure/preparedMeasureData';
import { partsAt, validateStabilityPartsReply } from './partsData';
import { isMember } from '../../dashboard/groups';
import { useParityRows } from '../../dashboard/parityRows';
import type { ParityList } from '../../dashboard/parityRows';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { validateMeasureStates } from '../country-measure/preparedMeasureData';
import type { Measure } from './stabilityData';
import { summaryStack } from '../../charts/chartSummary';
import type { ChartSummary } from '../../charts/chartSummary';

type Metadata = { parts?: Record<string, MeasureParts> };
const metadata = (data: (MeasureStates | MeasureSummaries) & Metadata | undefined) => {
  const parts = data?.parts ?? {};
  for (const [tag,measure] of Object.entries(parts)) validateStabilityPartsReply({branchId:data!.branchId,tag,decimals:data!.decimals,stability:measure,warSupport:measure,warnings:[]});
  return parts;
};
const partsPending = (data: MeasureStates | undefined, hour: number, within?: ReadonlyMap<string,readonly HourSpan[]>) => data?.states.some(state=>{
  const [tag,id]=JSON.parse(state.tag) as [string,string];
  const pending=(state as typeof state & {pending?:boolean}).pending;
  if(pending!==undefined && typeof pending!=='boolean') throw new Error('The prepared part pending flag is malformed.');
  return state.hour===hour && isMember(within?.get(tag),hour) && (state.approximate===true || !id && pending===true);
}) ?? false;
/** Runs keep legacy stack samples; dense buckets keep ordered extrema. */
export function preparedPartsGeometry(data: MeasureSummaries | undefined, tags: readonly string[] = Object.keys(metadata(data))): MeasureParts & { approximate?: ChartSummary['approximate'] } {
  const parts = new Map<string, MeasureParts['parts'][number]>(), observed: HourSpan[] = [];
  const names = metadata(data);
  const lines = data?.series ?? [];
  const stacked = summaryStack(lines.map(line => line.summary));
  for (const line of tags.flatMap(tag => lines.filter(line => (JSON.parse(line.tag) as [string,string])[0] === tag))) {
    const [tag, id] = JSON.parse(line.tag) as [string, string];
    const summary = line.summary;
    const spans = summary.runs?.map(run => ({firstHour: run.firstHour, lastHour: run.lastHour, startsSegment: !!run.breakBefore}))
      ?? summary.buckets.flatMap(bucket => bucket ? bucket.hasBreak ? [{firstHour: bucket.firstHour,lastHour: bucket.firstHour,startsSegment:true}, {firstHour: bucket.lastHour,lastHour: bucket.lastHour,startsSegment:true}]
        : [{firstHour: bucket.firstHour,lastHour: bucket.lastHour,startsSegment:bucket.firstBreak}] : []);
    if (!id) { for (const span of spans) { const last = observed.at(-1); if (last && !span.startsSegment && last.lastHour + 1 === span.firstHour) last.lastHour = span.lastHour; else observed.push({firstHour:span.firstHour,lastHour:span.lastHour}); } continue; }
    const metadata = names[tag]?.parts.find(part => part.id === id);
    if (!metadata) throw new Error('Prepared part metadata is missing.');
    const buckets = summary.buckets.filter(bucket=>bucket!==null);
    const runs = summary.runs?.filter(run=>run.firstHour<=summary.to && run.lastHour>=summary.from).map(run => {
      let low=0,high=buckets.length; while(low<high) {const middle=(low+high)>>1; if(buckets[middle].lastHour<run.firstHour) low=middle+1; else high=middle;}
      const bucket=buckets[low];
      if(!bucket || bucket.firstHour>run.lastHour) throw new Error('An exact part run has no hourly bucket.');
      const raw=BigInt((bucket as typeof bucket & {absoluteTotal:string}).absoluteTotal)*(bucket.firstValue<0 ? -1n : 1n);
      return {firstHour:run.firstHour,lastHour:run.lastHour,valueRaw:String(raw),startsSegment:!!run.breakBefore};
    })
      ?? (() => {
        // Parts omit zeros, so one absent child bucket cannot gate another
        // child's known contribution. Observed coverage gates the final stack.
        const { model, sample } = summaryStack([summary]);
        return model.hours.flatMap((hour,index) => {
          if (!model.bounds[index]) return [];
          const edge = sample(hour)[0], value = edge.low < 0 ? edge.low - edge.high : edge.high - edge.low;
          return [{firstHour:hour,lastHour:(model.hours[index+1] ?? hour+1)-1,valueRaw:String(value*10**data!.decimals),startsSegment:model.breakBefore[index]}];
        });
      })();
    const previous = parts.get(id); if (previous) previous.runs.push(...runs); else parts.set(id,{...metadata,runs});
  }
  const order=[...new Set(tags.flatMap(tag=>names[tag]?.parts.map(part=>part.id) ?? []))];
  return {parts:order.flatMap(id=>{const part=parts.get(id);return part ? [{...part,runs:part.runs.sort((a,b)=>a.firstHour-b.firstHour)}] : [];}),observed:observed.sort((a,b)=>a.firstHour-b.firstHour),...(stacked.approximate.length ? { approximate: stacked.approximate } : {})};
}
export function preparedPartsWeights(data: MeasureSummaries | undefined): Map<string,bigint> {
  const weights = new Map<string,bigint>();
  for (const line of data?.series ?? []) { const [,id] = JSON.parse(line.tag) as [string,string]; if (!id) continue;
    const total = line.summary.buckets.reduce((sum,bucket)=> { const raw = (bucket as (typeof bucket & {absoluteTotal:string}))?.absoluteTotal; if (bucket && !/^\d+$/.test(raw)) throw new Error('Prepared part contribution is missing.'); return sum + BigInt(raw ?? '0'); },0n);
    weights.set(id,(weights.get(id) ?? 0n)+total);
  }
  return weights;
}
export function preparedPartsAt(data: MeasureStates | undefined, hour: number, within?: ReadonlyMap<string,readonly HourSpan[]>, tags: readonly string[] = Object.keys(metadata(data))): MeasureParts {
  const result: MeasureParts = {parts:[],observed:[]}, names = metadata(data);
  const known = new Set(data?.states.filter(state=> {const [tag,id]=JSON.parse(state.tag) as [string,string]; return !id && state.hour===hour && state.run && !state.approximate && isMember(within?.get(tag),hour);}).map(state=>(JSON.parse(state.tag) as [string,string])[0]));
  if (known.size) result.observed.push({firstHour:hour,lastHour:hour});
  const values = new Map<string, NonNullable<MeasureStates['states'][number]['run']>>();
  for (const state of data?.states ?? []) {const [tag,id]=JSON.parse(state.tag) as [string,string]; if (id && known.has(tag) && state.hour===hour && state.run && !state.approximate) values.set(id,state.run);}
  const seen = new Set<string>();
  for (const tag of tags) for (const part of names[tag]?.parts ?? []) {
    if (seen.has(part.id)) continue; seen.add(part.id);
    const run = values.get(part.id); if (run) result.parts.push({...part,runs:[{...run,firstHour:hour,lastHour:hour}]});
  }
  if (result.parts.length !== values.size) throw new Error('Prepared part metadata is missing.');
  return result;
}
export function usePreparedParts(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined, tags: readonly string[], measure: Measure, hour: number, hover: number | null, extent: readonly [number,number], active: boolean, within?: ReadonlyMap<string,readonly HourSpan[]>) {
  const prepared = usePreparedMeasure(client,source,branchId,measure,tags,hour,hover,active,false,'stability-parts',undefined,within);
  const ranking = usePreparedMeasure(client,source,branchId,measure,tags,hour,null,active,false,'stability-parts',undefined,within,false);
  useEffect(()=> { ranking.onSummaryView({from:extent[0],to:extent[1],width:1}); },[ranking.owner,extent[0],extent[1]]);
  const revision = useContentRevision(client), previousRevision = useRef(revision);
  useEffect(()=>{if(previousRevision.current!==revision){previousRevision.current=revision;prepared.retry();ranking.retry();}},[revision]);
  const geometry = useMemo(()=>preparedPartsGeometry(prepared.summaries,tags),[prepared.summaries,prepared.owner]);
  const weights = useMemo(()=>preparedPartsWeights(ranking.summaries),[ranking.summaries]);
  const at = (at:number) => preparedPartsAt(prepared.states,at,within,tags);
  return {...prepared,geometry,weights,at,
    pendingAt:(hour:number)=>partsPending(prepared.states,hour,within),
    loadedAt:(hour:number)=>tags.filter(tag=>isMember(within?.get(tag),hour)).every(tag=>prepared.states?.states.some(state=>state.tag===JSON.stringify([tag,'']) && state.hour===hour)) && !!prepared.states,
error:prepared.error ?? ranking.error,ready:!!prepared.summaries && !!ranking.summaries, retry:()=>{prepared.retry();ranking.retry();}};
}

export function usePartsParity(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined, tags: readonly string[], measure: Measure, extent: readonly [number,number], legacy: MeasureParts, within?: ReadonlyMap<string,readonly HourSpan[]>) {
  const root = useRef<HTMLDivElement>(null), {requested,prepare} = useParityPreparation();
  const key = JSON.stringify([source?.id,source?.revision,branchId,tags,measure,extent,within && [...within]]);
  const [answer,setAnswer] = useState<{key:string;client:DashboardClient;model?:ParityList;error?: Failure}>();
  useEffect(()=> { if (!requested || !source || !branchId) return; const controller = new AbortController();
    void (async()=> {const rows:unknown[]=[];
      for (let first=Math.floor(extent[0]);first<=extent[1];first+=128) {
        const hours=Array.from({length:Math.min(128,Math.floor(extent[1])-first+1)},(_,index)=>first+index);
        const data=client.preparedQuery ? validateMeasureStates(await preparedSeriesQuery(client,source,'series-state',{branchId,family:{kind:'stability-parts',measure},reads:tags.flatMap(tag=>hours.filter(hour=>isMember(within?.get(tag),hour)).map(hour=>({tag,hour})))},controller.signal,true)) : undefined;
        for (const hour of hours) {if(partsPending(data,hour,within)) throw new Error('Stability parts are still being prepared.'); const values=partsAt(data ? preparedPartsAt(data,hour,within,tags) : legacy,hour); rows.push({hour,values:values?.map(({part,raw})=>({id:part.id,valueRaw:raw})).sort((a,b)=>a.id.localeCompare(b.id)) ?? null});}
      }
      if (!controller.signal.aborted) setAnswer({key,client,model:{kind:'chart',label:'Stability part values',complete:true,total:rows.length,rows}});
    })().catch(error=>{if(!controller.signal.aborted)setAnswer({key,client,error: asFailure(error)});}); return ()=>controller.abort();
  },[requested,key,client,legacy]);
  const current=answer?.key===key && answer.client===client ? answer : undefined;
  useParityRows(root,()=>current?.model ?? {kind:'chart',label:'Stability part values',complete:false,total:0,rows:[]},{prepare,ready:()=>!!current?.model,error:()=>current?.error ?? null});
  return root;
}
