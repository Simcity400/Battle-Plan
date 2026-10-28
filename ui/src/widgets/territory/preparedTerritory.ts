import { useCallback, useMemo, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { useFollowedProduct, useProduct } from '../../dashboard/products.ts';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { TerritoryMeasure, TerritoryParams, TerritoryReply, TerritoryRule, TerritorySeries, HourSpan } from '../../generated/dashboard';
import type { Run } from '../../charts/series';
import { overlayCountryIdentities } from '../../dashboard/countryIdentity.ts';
import { TERRITORY_VALUE_COUNT } from '../../dashboard/territoryMeasure.ts';
import { validateTerritoryReply, territoryColumn } from './territoryData.ts';
import { validateMeasureSummaries } from '../country-measure/preparedMeasureData.ts';
import type { MeasureSummaries } from '../country-measure/preparedMeasureData';

export function useTerritoryMetadata(client: DashboardClient, track: BoundTrack | undefined, active: boolean) {
  const request = track ? { sourceId:track.source.id,revision:track.source.revision,product:'series-state',
    params:{branchId:track.branch.id,family:{kind:'territory',metadata:true},reads:[{hour:track.branch.firstHour}]} } : null;
  return useProduct<TerritoryReply>(client,request,active,validateTerritoryReply,track?.source);
}

/** Only reached exact hours and the visible width summary are retained. */
export function usePreparedTerritory(client: DashboardClient, track: BoundTrack | undefined, selection: TerritoryParams | null,
  measure: TerritoryMeasure, rule: TerritoryRule, hour: number, active: boolean, within?: ReadonlyMap<string, readonly HourSpan[]>, aggregate=false, unobserved:readonly HourSpan[]=[]) {
  const ownerKey = JSON.stringify([track?.source.id,track?.source.revision,selection,measure,rule,within && [...within],aggregate,unobserved]);
  const owner = useMemo(()=>({client,key:ownerKey}),[client,ownerKey]);
  const [hover,setHover] = useState<{owner:typeof owner;hour:number|null}>();
  const [view,setView] = useState<{owner:typeof owner;from:number;to:number;width:number}>();
  const [navigation,setNavigation] = useState<{owner:typeof owner;params:unknown;revision:number}>();
  const family = {kind:'territory',measure,rule,top:selection?.top?.count ?? null,aggregate};
  const request = (product:string,extra:object) => track && selection ? {sourceId:track.source.id,revision:track.source.revision,product,
    params:{branchId:track.branch.id,family,tags:selection.tags,within:within && Object.fromEntries(within),unobserved,...extra}} : null;
  // The last hour's state stays shown while playback's newest hour is read.
  const head = useFollowedProduct<TerritoryReply>(client,request('series-state',{reads:[{hour}]}),['reads'],active,validateTerritoryReply,track?.source);
  const hovered = hover?.owner===owner ? hover.hour : null;
  const reached = useProduct<TerritoryReply>(client,hovered===null ? null : request('series-state',{reads:[{hour:hovered}]}),active,validateTerritoryReply,track?.source);
  const viewport = view?.owner===owner ? view : undefined;
  // One summary read at a time while the chart's view moves.
  const summaries = useFollowedProduct<MeasureSummaries>(client,viewport ? request('series-summary',{from:viewport.from,to:viewport.to,width:viewport.width}) : null,
    ['from','to','width'],active,validateMeasureSummaries,track?.source);
  const next = useProduct<{hour:number|null}>(client,navigation?.owner===owner && track ? {sourceId:track.source.id,revision:track.source.revision,product:'series-next-change',params:navigation.params} : null,
    active,value=>{const reply=value as {hour:number|null};if(reply?.hour!==null && (typeof reply?.hour!=='number' || !Number.isFinite(reply.hour))) throw new Error('The prepared territory navigation reply is malformed.');return reply;},track?.source);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(value=>{
    setView(before=>before?.owner===owner && before.from===value.from && before.to===value.to && before.width===value.width ? before : {owner,...value});
  },[owner]);
  const onHoverHour = useCallback((hour:number|null)=>{
    const at=hour===null ? null : Math.floor(hour);
    setHover(before=>before?.owner===owner && before.hour===at ? before : {owner,hour:at});
  },[owner]);
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((key,from,x,until)=>{
    const query=request('series-next-change',{key,from,firstHour:x[0],lastHour:Math.min(x[1],until ?? x[1])});
    if(query) setNavigation(before=>({owner,params:query.params,revision:(before?.revision ?? 0)+1}));
  },[owner]);
  const lineSummaries = useMemo(()=>new Map(summaries.data?.series.map(line=>[line.tag,line.summary]) ?? []),[summaries.data]);
  const data = useMemo(()=>mergeTerritoryReads(head.data,reached.data,summaries.data,territoryColumn(measure,rule),aggregate),[head.data,reached.data,summaries.data,measure,rule,aggregate]);
  const sumRuns=useMemo(()=>{
    const replies=[head.data,reached.data,summaries.data] as (TerritoryReply & {sum?:Run[]}|undefined)[];
    const runs=new Map(replies.flatMap(reply=>reply?.sum ?? []).map(run=>[JSON.stringify(run),run]));
    return [...runs.values()].sort((a,b)=>a.firstHour-b.firstHour);
  },[head.data,reached.data,summaries.data]);
  // The hour the shown head answers; one kept while playback's newest loads holds its own.
  return {headHour: head.data && head.moment ? (JSON.parse(head.moment) as [{ hour: number }[]])[0][0]?.hour : undefined, data,sumRuns,lineSummaries,onSummaryView,onHoverHour,onNavigateHour,owner,
    keyboardHour:next.data?.hour,keyboardRevision:navigation?.revision,
    error:head.error ?? reached.error ?? summaries.error ?? next.error,
    retry:()=>{head.retry();reached.retry();summaries.retry();next.retry();}};
}

export function mergeTerritoryReads(head:TerritoryReply|undefined,hover:TerritoryReply|undefined,summaries:MeasureSummaries|undefined,column:number,aggregate=false):TerritoryReply|undefined {
  if(!head) return undefined;
  const series = new Map<string,TerritorySeries>();
  for(const reply of [head,hover]) for(const line of reply?.series ?? []) {
    const previous=series.get(line.tag);
    const runs=[...(previous?.runs ?? []),...line.runs];
    const unique=new Map(runs.map(run=>[JSON.stringify(run),run]));
    series.set(line.tag,{...line,runs:[...unique.values()].sort((a,b)=>a.firstHour-b.firstHour)});
  }
  for(const line of summaries?.series ?? []) {
    if(aggregate && line.tag==='group') continue;
    const metadata=line as typeof line & Pick<TerritorySeries,'segments'|'identities'> & {endRuns?:TerritorySeries['runs']};
    const previous=series.get(line.tag);
    const ends=new Map([...(previous?.runs ?? []),...metadata.endRuns ?? []].map(run=>[JSON.stringify(run),run]));
    const exact=line.summary.approximate?.length ? undefined : line.summary.runs?.map(run=>({firstHour:run.firstHour,lastHour:run.lastHour,startsSegment:!!run.breakBefore,
      values:Array.from({length:TERRITORY_VALUE_COUNT},(_,index)=>index===column ? run.value : null)}));
    series.set(line.tag,{tag:line.tag,runs:exact ?? [...ends.values()].sort((a,b)=>a.firstHour-b.firstHour),segments:metadata.segments ?? previous?.segments ?? [],
      identities:metadata.identities ?? previous?.identities,membership:previous?.membership ?? null});
  }
  const native = (lines: readonly TerritorySeries[]) => lines.flatMap(line => line.identities ? [{ tag:line.tag,identities:line.identities }] : []);
  const exactIdentities = overlayCountryIdentities(native(head.series),native(hover?.series ?? []));
  for(const history of overlayCountryIdentities(native([...series.values()]),exactIdentities)) {
    const line = series.get(history.tag)!;
    series.set(history.tag,{...line,identities:history.identities,
      segments:history.identities.map(({firstHour,lastHour})=>({firstHour,lastHour}))});
  }
  const worlds=new Map([...head.worldTotals,...hover?.worldTotals ?? []].map(run=>[JSON.stringify(run),run]));
  return {...head,series:[...series.values()],worldTotals:[...worlds.values()].sort((a,b)=>a.firstHour-b.firstHour),
    warnings:[...new Set([...head.warnings,...hover?.warnings ?? [],...summaries?.warnings ?? []])]};
}
