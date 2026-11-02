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
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { TerritoryBreakdownReply } from '../../generated/dashboard';
import { measureOf, ruleOf, territoryColumn, territoryNote } from '../territory/territoryData';
import { breakdownItems, breakdownModel, breakdownSelectedWhole, breakdownWhole, mergeBreakdownReplies, validateBreakdownReply } from './breakdownData';
import { LEVELS, OPTIONS } from './definition';
import { useParityRows } from '../../dashboard/parityRows';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];
export default function TerritoryBreakdownWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const options = resolveOptions(OPTIONS, widget.options);
  const measure = measureOf(options.measure), rule = ruleOf(options.rule);
  const hour = window?.lastHour ?? 0, branchEnd = track?.branch.lastHour ?? 0;
  const end = widget.time.mode === 'range' ? hour : branchEnd;
  const request = (at:number) => track ? {sourceId:track.source.id,revision:track.source.revision,product:'series-state',params:{branchId:track.branch.id,
    family:{kind:'territory',measure,rule,breakdown:true},tags:[],reads:[{hour:Math.floor(at)}]}} : null;
  // The last hour stays shown while playback's newest hour is read.
  const current = useFollowedProduct<TerritoryBreakdownReply>(runtime.client,request(hour),['reads'],active,validateBreakdownReply,track?.source);
  const ending = useProduct<TerritoryBreakdownReply>(runtime.client,request(end),active,validateBreakdownReply,track?.source);
  // The hour the shown reply answers, which a retained one keeps while the next loads.
  const shown = current.data && current.moment ? (JSON.parse(current.moment) as [{ hour: number }[]])[0][0].hour : hour;
  const data = useMemo(()=>current.data && ending.data ? mergeBreakdownReplies(current.data,ending.data) : undefined,[current.data,ending.data]);
  const product={data,error:current.error ?? ending.error,retry:()=>{current.retry();ending.retry();}};
  const parityRoot=useRef<HTMLDivElement>(null);
  useParityRows(parityRoot,()=>({kind:'chart',label:'Territory regional values',complete:!!data,total:data?.series.length ?? 0,hour,
    rows:data?.series.map(series=>({tag:series.tag,region:series.region,run:runAt(series.runs,hour) ?? null})) ?? []}));
  const groups = useCountryGroups(runtime, track, active);
  const colours = useFactionColours(runtime.client, track?.source);
  const model = useMemo(() => product.data ? breakdownModel(product.data, measure, rule) : null, [product.data, measure, rule]);
  const identities = model?.identities ?? null;
  const fallback = defaultCountryTag(track);
  const filter = useMemo(() => worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, shown, branchEnd, identities),
    [widget.countries, fallback, groups.data, shown, branchEnd, identities]);
  const endFilter = useMemo(() => worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, end, branchEnd, identities),
    [widget.countries, fallback, groups.data, end, branchEnd, identities]);
  const countries = useMemo(() => new Map(product.data?.countries.map(country => [country.tag, country]) ?? []), [product.data]);
  const tags = useMemo(() => [...countries.keys()], [countries]);
  const segmentAt = useCallback((tag: string, at: number) => runAt(countries.get(tag)?.segments ?? [], at)?.firstHour ?? null, [countries]);
  const looks = useCountryLooks(runtime.client, track, tags, shown, active, segmentAt);
  const endLooks = useCountryLooks(runtime.client, track, tags, end, active, segmentAt);
  const includes = useCallback((tag: string | null, at: number) => { const chosen = at === end ? endFilter : filter;
    const key = tag === null ? null : identities?.keyAt(tag, at);
    return chosen.countryIds == null || key != null && chosen.countryIds.has(key); }, [end, filter, endFilter, identities]);
  const shape = useMemo(() => model && groups.data ? breakdownItems(model, groups.data, shown, end, {
    country: (tag, at) => {
      const atLooks = at === end ? endLooks : looks;
      return { name: atLooks.name(tag, at), flag: atLooks.flag(tag, at),
        flagPending: atLooks.flagPending(tag, at), flagReason: atLooks.flagReason(tag, at) };
    }, faction: colours.names,
    colour: id => colours.colours.get(id), selected: includes,
  }) : { items: [] }, [model, groups.data, shown, end, looks, endLooks, colours.names, colours.colours, includes]);
  const whole = model ? widget.countries.kind === 'all' ? breakdownWhole(model.data, shown, end, measure, rule)
    : breakdownSelectedWhole(model.data, shown, end, measure, rule, includes, model.identities) : undefined;
  const worldRuns = useMemo(() => product.data?.worldTotals.filter(run => run.values[territoryColumn(measure, rule)] !== null) ?? [], [product.data, measure, rule]);
  const worldStarts = useMemo(() => stretchStarts(worldRuns), [worldRuns]);
  const stretch = shape.items.some(item => item.present && (item.unknownValue || item.unknownLevels?.some(Boolean)))
    ? null : stretchAt(worldRuns, worldStarts, shown);
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
  if (product.error) return <WidgetState kind="error" failure={product.error} onRetry={product.retry} />;
  const groupFailure = groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!product.data || !groups.data || colours.loading) return <>{groupFailure}<WidgetState kind="loading" label="Loading territory breakdown..." /></>;
  if (colours.error) return <>{groupFailure}<WidgetState kind="error" what="faction colours" failure={colours.error} /></>;
  if (options.growth === 'fill' && endFilter.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={endFilter.reason} /></>;
  if (filter.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={filter.reason} /></>;
  if (measure === 'area' && product.data.areaReason) return <>{groupFailure}<WidgetState kind="unavailable" message="Map area is unavailable." /></>;
  if (!filter.known && !shape.items.some(item => item.present)) return groupFailure ?? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  return <div ref={parityRoot}>{groupFailure}<Sunburst key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label="Territory breakdown" items={shape.items}
    root={{ id: 'territory', label: widget.countries.kind === 'all' ? 'World territory' : 'Selected territory' }} authoritativeTotal={whole}
    stretch={stretch} unavailableReason={whole?.unknownValue ? 'Territory not recorded at this hour.' : undefined}
    levels={levelsShown(options, LEVELS)} text={text} settings={sunburstSettings(options)} active={active}
    hoursPerSecond={motionOf(runtime.clock.getSnapshot())} footnote={territoryNote(measure)}
    onContextMenu={(_node, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, shown)])} />{menu.menu}</div>;
}
