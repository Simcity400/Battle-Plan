import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { jumpItem } from '../../charts/jump';
import { runAt, stretchAt, stretchStarts } from '../../charts/series';
import { usNumber } from '../../charts/numberFormat';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { useFactionColours } from '../../dashboard/FactionColours';
import { MEMBERSHIP_PREPARING, worldFilter } from '../../dashboard/groups';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { TerritoryBreakdownReply } from '../../generated/dashboard';
import { measureOf, ruleOf, territoryColumn, territoryNote } from '../territory/territoryData';
import { breakdownItems, breakdownModel, breakdownSelectedWhole, breakdownWhole, validateBreakdownReply } from './breakdownData';
import { LEVELS, OPTIONS } from './definition';
import { useParityRows } from '../../dashboard/parityRows';

const NO_WARNINGS: string[] = [];
export default function TerritoryBreakdownWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const options = resolveOptions(OPTIONS, widget.options);
  const measure = measureOf(options.measure), rule = ruleOf(options.rule);
  const hour = window?.lastHour ?? 0, branchEnd = track?.branch.lastHour ?? 0;
  const end = widget.time.mode === 'range' ? hour : branchEnd;
  const request = (at:number) => track ? {sourceId:track.source.id,revision:track.source.revision,product:'series-state',params:{branchId:track.branch.id,
    family:{kind:'territory',measure,rule,breakdown:true},tags:[],reads:[{hour:Math.floor(at)}]}} : null;
  const current = useProduct<TerritoryBreakdownReply>(runtime.client,request(hour),active,validateBreakdownReply,track?.source);
  const ending = useProduct<TerritoryBreakdownReply>(runtime.client,request(end),active,validateBreakdownReply,track?.source);
  const data = useMemo(()=>{
    if(!current.data || !ending.data) return undefined;
    const mergeRuns=<T,>(a:T[],b:T[])=>[...new Map([...a,...b].map(run=>[JSON.stringify(run),run])).values()];
    const series=new Map(current.data.series.map(line=>[JSON.stringify([line.tag,line.region,line.missing]),line]));
    for(const line of ending.data.series){const key=JSON.stringify([line.tag,line.region,line.missing]);const before=series.get(key);series.set(key,{...line,runs:mergeRuns(before?.runs ?? [],line.runs).sort((a,b)=>a.firstHour-b.firstHour)});}
    const countries=new Map(current.data.countries.map(line=>[line.tag,line]));
    for(const line of ending.data.countries){const before=countries.get(line.tag);countries.set(line.tag,{...line,runs:mergeRuns(before?.runs ?? [],line.runs).sort((a,b)=>a.firstHour-b.firstHour)});}
    return {...current.data,series:[...series.values()],countries:[...countries.values()],worldTotals:mergeRuns(current.data.worldTotals,ending.data.worldTotals).sort((a,b)=>a.firstHour-b.firstHour),
      observed:current.data.observed.map((spans,index)=>mergeRuns(spans,ending.data!.observed[index]).sort((a,b)=>a.firstHour-b.firstHour)),warnings:[...new Set([...current.data.warnings,...ending.data.warnings])]};
  },[current.data,ending.data]);
  const product={data,error:current.error ?? ending.error,retry:()=>{current.retry();ending.retry();}};
  const parityRoot=useRef<HTMLDivElement>(null);
  useParityRows(parityRoot,()=>({kind:'chart',label:'Territory regional values',complete:!!data,total:data?.series.length ?? 0,hour,
    rows:data?.series.map(series=>({tag:series.tag,region:series.region,run:runAt(series.runs,hour) ?? null})) ?? []}));
  const groups = useCountryGroups(runtime, track, active);
  const colours = useFactionColours(runtime.client, track?.source);
  const model = useMemo(() => product.data ? breakdownModel(product.data, measure, rule) : null, [product.data, measure, rule]);
  const identities = model?.identities ?? null;
  const fallback = defaultCountryTag(track);
  const filter = useMemo(() => worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, hour, branchEnd, identities),
    [widget.countries, fallback, groups.data, hour, branchEnd, identities]);
  const endFilter = useMemo(() => worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, end, branchEnd, identities),
    [widget.countries, fallback, groups.data, end, branchEnd, identities]);
  const countries = useMemo(() => new Map(product.data?.countries.map(country => [country.tag, country]) ?? []), [product.data]);
  const tags = useMemo(() => [...countries.keys()], [countries]);
  const segmentAt = useCallback((tag: string, at: number) => runAt(countries.get(tag)?.segments ?? [], at)?.firstHour ?? null, [countries]);
  const looks = useCountryLooks(runtime.client, track, tags, hour, active, segmentAt);
  const endLooks = useCountryLooks(runtime.client, track, tags, end, active, segmentAt);
  const includes = useCallback((tag: string | null, at: number) => { const chosen = at === end ? endFilter : filter;
    const key = tag === null ? null : identities?.keyAt(tag, at);
    return chosen.countryIds == null || key != null && chosen.countryIds.has(key); }, [end, filter, endFilter, identities]);
  const shape = useMemo(() => model && groups.data ? breakdownItems(model, groups.data, hour, end, {
    country: (tag, at) => {
      const atLooks = at === end ? endLooks : looks;
      return { name: atLooks.name(tag, at), flag: atLooks.flag(tag, at),
        flagPending: atLooks.flagPending(tag, at), flagReason: atLooks.flagReason(tag, at) };
    }, faction: colours.names,
    colour: id => colours.colours.get(id), selected: includes,
  }) : { items: [] }, [model, groups.data, hour, end, looks, endLooks, colours.names, colours.colours, includes]);
  const whole = model ? widget.countries.kind === 'all' ? breakdownWhole(model.data, hour, end, measure, rule)
    : breakdownSelectedWhole(model.data, hour, end, measure, rule, includes, model.identities) : undefined;
  const worldRuns = useMemo(() => product.data?.worldTotals.filter(run => run.values[territoryColumn(measure, rule)] !== null) ?? [], [product.data, measure, rule]);
  const worldStarts = useMemo(() => stretchStarts(worldRuns), [worldRuns]);
  const stretch = shape.items.some(item => item.present && (item.unknownValue || item.unknownLevels?.some(Boolean)))
    ? null : stretchAt(worldRuns, worldStarts, hour);
  const text = useMemo<SunburstText>(() => ({ format: (value, exact, denominator = 1n) => {
    // At most three decimals: the en-US default.
    if (exact == null) return usNumber.format(value);
    const whole = exact / denominator, remainder = exact % denominator;
    return remainder === 0n ? usNumber.format(whole)
      : `${whole === 0n ? '' : `${usNumber.format(whole)} `}${remainder}/${denominator}`;
  },
    unit: measure === 'area' ? product.data?.areaUnit ?? 'map cells' : measure === 'victoryPoints' ? 'victory points' : measure,
    valueLabel: rule === 'owned' ? 'Owned' : 'Controlled', missingLabel: 'Not recorded', emptyLabel: 'No territory at this hour.' }), [measure, rule, product.data?.areaUnit]);
  const menu = useContextMenu('Territory breakdown actions');
  useEffect(() => { const warnings = [...(product.data?.warnings ?? []), ...(groups.data?.warnings ?? []), ...colours.warnings];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS); return () => onDiagnostics(NO_WARNINGS);
  }, [product.data, groups.data, colours.warnings, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign to see its territory breakdown." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (product.error) return <WidgetState kind="error" detail={product.error} onRetry={product.retry} />;
  if (groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (!product.data || !groups.data || colours.loading) return <WidgetState kind="loading" label="Loading territory breakdown..." />;
  if (colours.error) return <WidgetState kind="error" what="faction colours" detail={colours.error} />;
  if (options.growth === 'fill' && endFilter.reason) return <WidgetState kind="unavailable" message={endFilter.reason} />;
  if (filter.reason) return <WidgetState kind="unavailable" message={filter.reason} />;
  if (measure === 'area' && product.data.areaReason) return <WidgetState kind="unavailable" message="Map area is unavailable." />;
  if (!filter.known && !shape.items.some(item => item.present)) return <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  return <div ref={parityRoot}><Sunburst key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label="Territory breakdown" items={shape.items}
    root={{ id: 'territory', label: widget.countries.kind === 'all' ? 'World territory' : 'Selected territory' }} authoritativeTotal={whole}
    stretch={stretch} unavailableReason={whole?.unknownValue ? 'Territory not recorded at this hour.' : undefined}
    levels={levelsShown(options, LEVELS)} text={text} settings={sunburstSettings(options)} active={active}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} footnote={territoryNote(measure)}
    onContextMenu={(_node, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />{menu.menu}</div>;
}
