import { asFailure } from '../../dashboard/queryError.ts';
import { type Failure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useProductWindows, useWindowDisplay } from '../../dashboard/productWindows';
import { preparedSeriesQuery, preparedWindowQuery } from '../../dashboard/preparedSeries';
import type { PreparedBinding } from '../../dashboard/preparedSeries';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useViewRead } from '../../dashboard/viewRead';
import type { NavalLossesParams, NavalLossesReply } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import { chartSummaryLines } from '../../charts/chartSummary';
import type { TimeChartProps } from '../../charts/TimeChart';
import { battleWindowIndex } from '../land-battles/landBattlesData';
import { navalModel, navalValue, mergeNavalWindows, validateNavalWindow, validateNavalManifest } from './navalLossesData.ts';
import type { NavalManifest } from './navalLossesData';

export interface SummaryReply { series: { tag: string; summary: ChartSummary }[]; sampled?: NavalLossesReply }
function validateSummary(value: unknown): SummaryReply {
  const reply = value as SummaryReply;
  const series = chartSummaryLines<SummaryReply['series'][number]>(reply?.series, line => typeof line.tag === 'string');
  if (!series) throw Error('The naval loss summary is malformed.');
  if (reply.sampled) validateNavalWindow(reply.sampled);
  return { ...reply, series };
}
export function usePreparedNaval(client: DashboardClient, track: BoundTrack | undefined, params: NavalLossesParams | null,
  moment: number, extent: readonly [number,number], view: 'race' | 'lines' | 'mix', active: boolean, parity = false) {
  const content = useContentRevision(client);
  const identity = track && params ? JSON.stringify([track.source.id,track.source.revision,params,content,view,extent]) : null;
  const owner = useMemo(() => ({ client,identity }),[client,identity]);
  const alive = useRef(owner); alive.current = owner;
  const pending = useRef(new Set<AbortController>());
  const [navigationError,setNavigationError] = useState<{owner: typeof owner; error: Failure}>();
  const [hover,setHover] = useState<{owner: typeof owner; hour: number | null}>();
  const [viewport,setViewport] = useState<{owner: typeof owner; from: number; to: number; width: number; series: PreparedBinding[]}>();
  useEffect(() => () => { for (const controller of pending.current) controller.abort(); pending.current.clear(); },[owner,active]);
  const manifest = useProduct<NavalManifest>(client,track && params ? { sourceId:track.source.id,revision:track.source.revision,product:'naval-losses',params } : null,
    active,validateNavalManifest,track?.source);
  const geometry = viewport?.owner === owner ? viewport : undefined;
  const dense = view !== 'race' && !!geometry && Math.floor(geometry.to)-Math.floor(geometry.from)+1 > geometry.width;
  const [attempt,setAttempt] = useState(0);
  const summaryKey = JSON.stringify([geometry?.from,geometry?.to,geometry?.width,geometry?.series]);
  const pickOwner = useMemo(()=>({owner,summaryKey}),[owner,summaryKey]);
  const pickAlive = useRef(pickOwner); pickAlive.current=pickOwner;
  const sequence = useRef(0);
  const [keyboard,setKeyboard] = useState<{owner: typeof pickOwner; hour: number | null; revision: number}>();
  // One summary read at a time per series selection while the chart's view moves (`useViewRead`).
  const incoming = useViewRead<NonNullable<typeof geometry>,SummaryReply>(identity && geometry && dense ? JSON.stringify([identity,geometry.series]) : null,
    geometry,JSON.stringify([geometry?.from,geometry?.to,geometry?.width]),active,
    (at,signal)=>preparedWindowQuery(client,track!.source,'naval-losses',{branchId:params!.branchId,tags:params!.tags,parts:view==='mix',
      family:{navalKind:params!.kind,attribution:params!.attribution,period:params!.period,top:params!.top}},at,signal,validateSummary,
      (first,second)=>({series:[...first.series,...second.series],sampled:first.sampled&&second.sampled?mergeNavalWindows(first.sampled,[first.sampled,second.sampled]):first.sampled??second.sampled}),attempt>0), client);
  const summaries = useMemo(() => new Map(incoming?.data?.series.map(line => [line.tag,line.summary])),[incoming?.data]);
  const reached = [...new Set([moment,...(hover?.owner === owner && hover.hour !== null ? [hover.hour] : []),
    ...(keyboard?.owner === pickOwner && keyboard.hour !== null ? [keyboard.hour] : []),
    ...[...summaries.values()].flatMap(summary => { for (let index=summary.buckets.length-1;index>=0;index--) if(summary.buckets[index]) return [summary.buckets[index]!.lastHour]; return []; })])];
  const windows = manifest.data?.windows ?? [];
  // The windows shown or reached; the next one is only read ahead, so its failure waits until it is reached.
  const required = new Set(reached.map(hour => battleWindowIndex(windows,hour)).filter(index => index >= 0));
  if (parity || !dense && geometry && view !== 'race') windows.forEach((window,index) => {
    if (window.firstHour <= (parity ? extent[1] : geometry!.to) && window.lastHour >= (parity ? extent[0] : geometry!.from)) required.add(index);
  });
  const wanted = new Set(required);
  const next = battleWindowIndex(windows,moment)+1;
  if (view === 'race' && next > 0 && next < windows.length) wanted.add(next);
  const indices = [...wanted].sort((a,b)=>a-b);
  const history = useProductWindows<NavalLossesReply>(client,identity,index => ({ sourceId:track!.source.id,revision:track!.source.revision,product:'naval-losses',params:{...params!,window:index} }),
    indices,active && manifest.data?.available === true,validateNavalWindow,track?.source,true);
  const combined = useMemo(() => manifest.data ? mergeNavalWindows(manifest.data,history.data.values()) : undefined,[manifest.data,history.data]);
  const at = battleWindowIndex(windows,moment);
  const failed = history.errors.get(at) ?? [...required].sort((a,b)=>a-b).map(index => history.errors.get(index)).find(error => error !== undefined);
  const display = useWindowDisplay(identity,manifest.data && (at < 0 || !manifest.data.available || history.data.has(at)) ? combined : undefined,moment,client);
  const onSummaryView = useCallback((next: {from:number;to:number;width:number},series: PreparedBinding[]) => setViewport(previous => previous?.owner===owner
    && previous.from===next.from&&previous.to===next.to&&previous.width===next.width&&JSON.stringify(previous.series)===JSON.stringify(series)?previous:{owner,...next,series}),[owner]);
  const onReachHour = useCallback((hour: number | null) => setHover(previous => previous?.owner === owner && previous.hour === hour ? previous : {owner,hour:hour === null ? null : Math.floor(hour)}),[owner]);
  const exactLineTags = useCallback(async (hour: number) => {
    if (!active || !track || !params) return null;
    const controller = new AbortController(); pending.current.add(controller);
    try {
      const reply = await loadProduct(client,{sourceId:track.source.id,revision:track.source.revision,product:'naval-losses',params:{...params,hour:Math.floor(hour)}},controller.signal,validateNavalWindow,track.source);
      if (controller.signal.aborted || alive.current !== owner) return null;
      onReachHour(hour);
      return navalModel(reply).tags.filter(tag => reply.series.find(series=>series.tag===tag)?.runs.some(run=>navalValue(run,params.kind)!==null));
    } catch (error) { if (!controller.signal.aborted && alive.current === owner) setNavigationError({owner,error: asFailure(error)}); return null; }
    finally { pending.current.delete(controller); }
  },[owner,active,params,track]);
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((action,from,x,until) => {
    if (!active || !track || !params) return;
    const controller = new AbortController(); pending.current.add(controller);
    const request = ++sequence.current;
    void preparedSeriesQuery(client,track.source,'series-next-change',{branchId:params.branchId,tags:params.tags,
      family:{kind:'naval-losses',navalKind:params.kind,attribution:params.attribution,period:params.period,top:params.top},
      ...(geometry ? {within:Object.fromEntries(geometry.series.map(({id,spans})=>[id,spans.map(span=>
        [span.tag,span.firstHour,span.lastHour,span.startsSegment])]))} : {}),
      key:action,from,firstHour:x[0],lastHour:Math.min(x[1],until??x[1])},controller.signal).then(value=>{
      const hour = (value as {hour:number|null})?.hour;
      if (!(hour===null || Number.isSafeInteger(hour))) throw Error('The naval loss change reply is malformed.');
      if (!controller.signal.aborted && pickAlive.current === pickOwner && sequence.current === request) setKeyboard(previous=>({owner:pickOwner,hour,revision:(previous?.revision??0)+1}));
    }).catch(error=> {if(!controller.signal.aborted && pickAlive.current===pickOwner && sequence.current===request)setNavigationError({owner,error: asFailure(error)});}).finally(()=>pending.current.delete(controller));
  },[pickOwner,active,params,track,geometry]);
  return { data:display?.data,hour:display?.hour??moment,manifest:manifest.data,dense,summaries,sampled:incoming?.data?.sampled,
    onSummaryView,onReachHour,exactLineTags,onNavigateHour,keyboardHour:keyboard?.owner===pickOwner?keyboard.hour:undefined,
    keyboardRevision:keyboard?.owner===pickOwner?keyboard.revision:undefined,complete:indices.every(index=>history.data.has(index)),
    error:manifest.error??failed??incoming?.error??(navigationError?.owner===owner?navigationError.error:undefined),
    retry:()=>{manifest.retry();history.retry();setAttempt(attempt=>attempt+1);incoming.retry();setNavigationError(undefined);} };
}
