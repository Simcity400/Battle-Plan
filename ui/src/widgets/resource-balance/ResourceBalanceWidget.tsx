import { createContext, useContext, useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { Select } from '../../components/Select';
import '../../components/ChipSet.css';
import type { CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import DrillHost from '../../charts/drill/DrillHost';
import DrillParts from '../../charts/drill/DrillParts';
import { CLOSED, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel, drawGapFill, drawLine } from '../../charts/draw';
import { easeTau } from '../../charts/easing';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { pickLines } from '../../charts/linePick';
import { lineStyles } from '../../charts/palette';
import { clipToSpans, extentOfAll, runAt, sameStretch } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import { tooltipText } from '../../charts/tooltip';
import { useRowTooltips } from '../../charts/useRowTooltips';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { fixedNumber, formatAmount, formatSigned } from '../../charts/valueAxis';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { NameAt } from '../../dashboard/countryNamesAt';
import type { WidgetProps } from '../../dashboard/definitions';
import CountryComparisons from '../../dashboard/CountryComparisons';
import type { ComparisonProps } from '../../dashboard/CountryComparisons';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack, CountrySelection } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import type { ResourceBalanceReply, ResourceInfo } from '../../generated/dashboard';
import { balanceOf, barScale, changeHours, chosenLines, chosenResources, dealsKnown, orderResources, overTimeTooltip, resourceName,
  resourcePartGroups, resourceParts, resourceTooltip, supplyDemandRuns, toggleChosen, tooltipPartners, tradeIndex, tradesAt } from './balanceData';
import type { Balance, RowOrder } from './balanceData';
import { BREAKDOWN, OPTIONS } from './definition';
import { useResourceBalance, useResourceCountry } from './useResourceBalance';
import { clipResourceTrades, readResourceCountry, resourceCountryToken, resourceExtent, resourceMoment } from './resourceDrill';
import type { ResourceCountrySelection } from './resourceDrill';
import ResourceDeals from './ResourceDealsView';
import './resourceBalance.css';

const NO_WARNINGS: string[] = [];
const ResourceGroup = createContext<CountrySelection | null>(null);
type PartnerTargets = (hour: number | null, content?: TooltipContent | null) => void;

/** One country's resources: per resource, supply (produced, then imported) against
 * demand (needed, then exported) with the game's surplus or deficit, or supply and
 * demand over time. The hover lists each trade partner. */
export default function ResourceBalanceWidget(props: WidgetProps) {
  return props.widget.options?.show === BREAKDOWN.id && (props.widget.options.resourceCountry
    || props.widget.countries.kind === 'tags' && props.widget.countries.tags.length === 1)
    ? <ResourceBalanceCountry {...props} /> : <ResourceGroup.Provider value={props.widget.countries}><CountryComparisons {...props} component={ResourceBalanceCountry} /></ResourceGroup.Provider>;
}

function ResourceBreakdown({ data, resource, hour, tokens, runtime, track, ...chart }: {
  data: ResourceBalanceReply; resource: ResourceInfo; hour: number | null; tokens: ChartTokens; runtime: WidgetProps['runtime']; track: BoundTrack;
} & Omit<Parameters<typeof BreakdownChart>[0], 'series' | 'axes' | 'label'>) {
  const recorded = data.series.find(series => series.id === resource.id);
  const parts = useMemo<BreakdownSeries[]>(() => {
    const sample = recorded?.runs[0] && resourceParts(recorded.runs[0]);
    if (!sample) return [];
    return sample.map((part, index) => ({ id: part.id, label: part.label, color: tokens.categories[index % tokens.categories.length],
      runs: recorded!.runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, breakBefore: run.startsSegment,
        value: fixedNumber(resourceParts(run)[index].raw, data.decimals) })),
      amount: at => {
        const run = runAt(recorded!.runs, at);
        return run ? formatSigned(resourceParts(run)[index].raw, data.decimals, data.decimals) : 'Not recorded';
      },
    }));
  }, [recorded, tokens.categories, data.decimals]);
  const run = hour !== null && recorded ? runAt(recorded.runs, hour) : null;
  const balance = run && balanceOf(run);
  const values = run && resourceParts(run);
  const groups = run ? resourcePartGroups(run) : [];
  const deals = useMemo(() => hour !== null && dealsKnown(data, hour) ? tradesAt(tradeIndex(data), resource.id, hour) : null,
    [data, resource.id, hour]);
  return <>
    {groups.map(group => <div key={group.title}>
      <DrillParts parts={group.values.map(part => ({ id: part.id, label: part.label, color: parts.find(series => series.id === part.id)?.color,
        value: fixedNumber(part.raw, data.decimals), text: formatAmount(part.raw, data.decimals, data.decimals) }))}
        format={String} total={fixedNumber(group.total, data.decimals)} totalRow={{ label: group.title, value: formatAmount(group.total, data.decimals, data.decimals) }} />
    </div>)}
    {values && balance && values.at(-1)!.raw !== 0n && <DrillParts parts={[{ id: 'unidentified', label: 'Other', muted: true,
      value: fixedNumber(values.at(-1)!.raw, data.decimals), text: formatSigned(values.at(-1)!.raw, data.decimals, data.decimals), color: parts.at(-1)?.color }]}
      total={fixedNumber(balance.net, data.decimals)} format={String} />}
    {balance ? <p className="drill-empty">Net: {formatSigned(balance.net, data.decimals, data.decimals)} per day.</p>
      : <p className="drill-empty">No resources were recorded for this moment.</p>}
    {deals && hour !== null ? (['imports', 'exports'] as const).map(direction => {
      const pool = balance ? direction === 'imports' ? balance.imported : balance.exported : null;
      return <ResourceDeals key={direction} deals={deals[direction]} pool={pool} decimals={data.decimals} hour={hour}
        direction={direction} runtime={runtime} track={track} active={chart.active} />;
    }) : <p className="drill-empty">Trade partners were not recorded at this hour.</p>}
    <BreakdownChart {...chart} series={parts} selectedHour={hour} label={`${resourceName(resource)} parts over time`}
      axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} />
  </>;
}

function ResourceBalanceCountry({ widget, runtime, active, countrySlot, onDiagnostics, setOption, within }: ComparisonProps) {
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const clockHour = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data: reply, numbers, contentPending, error, retry } = useResourceBalance(widget, runtime, active);
  const data = useMemo(() => reply && within ? { ...reply, series: reply.series.map(series => ({ ...series, runs: clipToSpans(series.runs, within) })),
    trades: clipResourceTrades(reply.trades, within), dealHours: clipToSpans(reply.dealHours, within) } : reply, [reply, within]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const saved = options.show === BREAKDOWN.id;
  const savedCountry = useMemo<ResourceCountrySelection | null>(() => {
    if (!saved || !track) return null;
    if (options.resourceCountry) return readResourceCountry(options.resourceCountry, track.source.metadata.campaignKey, track.branch.id, widget.countries);
    return widget.time.mode === 'pin' && window && widget.countries.kind === 'tags' && widget.countries.tags.length === 1
      ? { campaign: track.source.metadata.campaignKey, branch: track.branch.id, tag: widget.countries.tags[0], originOffset: window.lastHour - track.branch.firstHour } : null;
  }, [saved, options.resourceCountry, widget.countries, widget.time.mode, window?.lastHour, track?.source.metadata.campaignKey, track?.branch.id, track?.branch.firstHour]); // eslint-disable-line react-hooks/exhaustive-deps
  const heldExtent = track ? resourceExtent(savedCountry, track.branch.firstHour, track.branch.lastHour) : null;
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = heldExtent ?? (track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1]);
  const head = Math.floor(playhead ?? extent[1]);
  const anchor = useRef<HTMLDivElement>(null);
  const [resourceId, setResourceId] = useState<number>();
  const savedKey = contentKey(String(options.partResource ?? ''));
  const picked = saved && savedKey ? data?.resources.find(resource => JSON.stringify([resource.token, resource.ordinal]) === savedKey)
    : data?.resources.find(resource => resource.id === resourceId) ?? data?.resources[0];
  const actualHour = track ? Math.floor(clockHour) + track.axisOffset : 0;
  const drillAt = resourceMoment(drill, saved, head, actualHour);
  const [countryPick, setCountryPick] = useState<ResourceCountrySelection | null>(null);
  const selectedCountry = saved ? savedCountry : countryPick;
  const originalGroup = useContext(ResourceGroup);
  const actor = useResourceCountry(selectedCountry, runtime, track, active, originalGroup);
  const detailData = selectedCountry ? actor.data : data;
  const headerTag = selectedCountry && actor.spans && drillAt !== null ? countryDrillTagAt(actor.spans, drillAt) : selectedCountry ? null : tag;
  const headerReason = saved && !savedCountry ? 'Saved country selection unavailable. Choose it again.'
    : selectedCountry ? actor.reason ?? (headerTag ? null : 'Country unavailable at this time.') : null;
  const header = useCountryLabelsAt(runtime.client, track, saved && countrySlot && !headerReason && headerTag && drillAt !== null
    ? [{ tag: headerTag, hour: drillAt }] : [], active);
  const pinnedHeader = saved && countrySlot ? createPortal(<DatedCountryChip hour={drillAt} reason={headerReason}
    label={!headerReason && headerTag && drillAt !== null ? header.labelAt(drillAt)(headerTag) : null} />, countrySlot) : null;
  const detailScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, data?.tag, picked?.id, options.show]);
  const selectedCountryKey = JSON.stringify(selectedCountry);
  const detailPickOwner = useMemo(() => ({ scope: detailScope, numbers: selectedCountry ? actor.numbers : numbers, within, selectedCountryKey }),
    [detailScope, selectedCountryKey, actor.numbers, numbers, within]);
  useEffect(() => { dispatch({ type: 'close' }); setResourceId(undefined); setCountryPick(null); },
    [runtime.client, track?.source.id, track?.source.revision, track?.branch.id, tag, options.show]);
  const [wantedPartners, setWantedPartners] = useState<NameAt[]>([]);
  const onPartners = useCallback<PartnerTargets>((hour, content) => {
    const wanted = hour === null ? [] : tooltipPartners(content ?? null).map(tag => ({ tag, hour }));
    setWantedPartners(previous => previous.length === wanted.length && previous.every((item, index) =>
      item.tag === wanted[index].tag && item.hour === wanted[index].hour) ? previous : wanted);
  }, []);
  const partners = useCountryLabelsAt(runtime.client, track, saved || drill.breakdown ? [] : wantedPartners, active);
  const menu = useContextMenu('Resource actions');
  useEffect(() => {
    onDiagnostics(contentPending ? ['Resource names and artwork are updating from the recorded content.']
      : data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, contentPending, onDiagnostics]);
  // A widget out of view shows no tooltip, so it needs no partner names either.
  useEffect(() => { if (!active) onPartners(null); }, [active, onPartners]);
  // One resource's breakdown at an hour. It changes only with the reply, the partners'
  // names and the window, so the breakdowns built for an hour are reused while the
  // pointer moves; the deals come from the reply's index, never a scan.
  const breakdown = useCallback((resource: ResourceInfo, hour: number): TooltipContent | null => {
    if (!data) return null;
    const series = data.series.find(item => item.id === resource.id);
    const run = series && runAt(series.runs, hour);
    if (!run) return { title: resourceName(resource), icon: resource.icon ?? undefined, subtitle: dayLabel(hour), rows: [{ label: 'Not recorded at this time', tone: 'muted' }] };
    // As much as fits the window, with the surplus or deficit always shown.
    return resourceTooltip({ title: resourceName(resource), icon: resource.icon ?? undefined, subtitle: `Per day · ${dayLabel(hour)}` },
      balanceOf(run), dealsKnown(data, hour) ? tradesAt(tradeIndex(data), resource.id, hour) : null, data.decimals, partners.labelAt(hour), viewport);
  }, [data, partners.labelAt, viewport]);

  if (!track || !tag) return <>{pinnedHeader}<WidgetState kind="empty" icon="layers" message="Load a campaign and choose a country to see its resources." /></>;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <>{pinnedHeader}<WidgetState kind="unavailable" message="This time is outside the recording." /></>;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Choose another time in widget settings." /></>;
  if (error) return <>{pinnedHeader}<WidgetState kind="error" detail={error} onRetry={retry} /></>;
  if (!data) return <>{pinnedHeader}<WidgetState kind="loading" label="Loading resources…" /></>;
  if (!data.series.some(series => series.runs.length) || !data.resources.length) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Resources weren’t recorded for this country." /></>;

  const jump = (hour: number) => [jumpItem(runtime, track.id, hour)];
  const open = (resource: ResourceInfo, hour: number) => {
    setResourceId(resource.id);
    setCountryPick({ campaign: track.source.metadata.campaignKey, branch: track.branch.id, tag, originOffset: hour - track.branch.firstHour,
      extent: extent.map(at => at - track.branch.firstHour) as [number, number] });
    dispatch({ type: 'open', breakdown: BREAKDOWN.id, hour, playing: runtime.clock.getSnapshot().playing,
      playhead: widget.time.mode === 'follow' ? playhead : null });
  };

  const main = <>
    {options.view === 'time'
      ? <OverTime data={data} tokens={tokens} viewport={viewport} options={options} runtime={runtime} track={track} extent={extent} playhead={playhead} active={active}
        numbers={numbers} within={within}
        partnerAt={partners.labelAt} partnersRevision={partners.revision} onPartners={onPartners}
        onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} setOption={setOption} onPick={open} />
      : <BalanceBars data={data} tokens={tokens} hour={head} order={options.order as RowOrder} scale={options.scale === 'moment' ? 'moment' : 'campaign'}
        hideEmpty={options.hideEmpty === true} owner={`resource-balance:${widget.id}:${tag}`} breakdown={breakdown} onPartners={onPartners} active={active}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} onPick={open} />}
  </>;
  const detail = saved && !savedCountry ? <WidgetState kind="unavailable" message={headerReason!} />
    : selectedCountry && actor.error ? <WidgetState kind="error" detail={actor.error} onRetry={actor.retry} />
    : selectedCountry && actor.reason ? <WidgetState kind="unavailable" message="Country unavailable at this time." />
    : picked && detailData ? <ResourceBreakdown data={detailData} resource={picked} hour={drillAt} tokens={tokens}
    extent={extent} playhead={drill.breakdown !== null && drill.follow ? actualHour : playhead} playback={options.playback as PlaybackMode} afterPlayhead={options.afterPlayhead === true}
    windowHours={Number(options.windowDays ?? 365) * 24} branchStart={track.branch.firstHour} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
    active={active} runtime={runtime} track={track} pickOwner={detailPickOwner} onPick={hour => dispatch(saved
      ? { type: 'open', breakdown: BREAKDOWN.id, hour, playing: false, playhead: null } : { type: 'pick', hour })}
    onContextMenu={(hour, x, y) => menu.open(x, y, jump(hour))} /> : <p className="drill-empty">Resource unavailable. Choose another in settings.</p>;
  return <div ref={anchor} className="resource-balance">
    {saved ? <div className="drill-body">
      {pinnedHeader}
      {(drill.breakdown === null ? widget.time.mode !== 'follow' : !drill.follow) && <button type="button" className="button small quiet" onClick={() => dispatch({ type: 'open', breakdown: BREAKDOWN.id,
        hour: actualHour, playing: true, playhead: actualHour })}>Follow playback</button>}{detail}
    </div> : <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null}
      title={picked ? resourceName(picked) : 'Resource breakdown'} day={drillAt === null ? '' : dayLabel(drillAt)} following={drill.follow} canFollow
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}
      onPin={picked && selectedCountry && !actor.reason ? () => {
        const pinned = pinnedBreakdown(widget, { ...BREAKDOWN, label: resourceName(picked) }, track.id);
        pinned.options = { ...pinned.options, partResource: setToken(JSON.stringify([picked.token, picked.ordinal])), resourceCountry: resourceCountryToken(selectedCountry) };
        if (!drill.follow && drillAt !== null) pinned.time = { mode: 'pin', windows: { [track.id]: { firstOffset: drillAt - track.branch.firstHour, lastOffset: drillAt - track.branch.firstHour } } };
        if (drill.follow) pinned.time = { mode: 'follow' };
        runtime.addWidgetBelow(widget.id, pinned);
      } : undefined}>
      <div className="select-row">Resource <Select label="Resource" value={String(picked?.id ?? '')} onChange={value => setResourceId(Number(value))}
        options={data.resources.map(resource => ({ value: String(resource.id), label: resourceName(resource) }))} /></div>
      {detail}
    </DrillHost>}
    {menu.menu}
  </div>;
}

/** Per resource, a supply bar above a demand bar on a shared scale, with the net at
 * the right. Rows are focusable: hover or focus shows the
 * breakdown, right-click or the context-menu key jumps to the hour shown. */
function BalanceBars({ data, tokens, hour, order, scale, hideEmpty, owner, breakdown, onPartners, active, hoursPerSecond, onMenu, onPick }: {
  data: ResourceBalanceReply; tokens: ChartTokens; hour: number; order: RowOrder; scale: 'campaign' | 'moment'; hideEmpty: boolean; owner: string;
  breakdown: (resource: ResourceInfo, hour: number) => TooltipContent | null; onPartners: PartnerTargets; active: boolean;
  hoursPerSecond: number; onMenu: (hour: number, x: number, y: number) => void;
  onPick: (resource: ResourceInfo, hour: number) => void;
}) {
  const campaignScale = useMemo(() => barScale(data.series, null), [data]);
  const largest = scale === 'moment' ? barScale(data.series, hour) : campaignScale;
  // The hour drawn last, so a bar glides only from a value of the same stretch of recording.
  const drawnHour = useRef(hour);
  useEffect(() => { drawnHour.current = hour; }, [hour]);
  const rows = orderResources(data, hour, order).map(resource => {
    const series = data.series.find(item => item.id === resource.id);
    const run = series ? runAt(series.runs, hour) : null;
    return { resource, balance: run ? balanceOf(run) : null, glides: series != null && sameStretch(series.runs, drawnHour.current, hour) };
  }).filter(row => !hideEmpty || !row.balance || row.balance.supply !== 0n || row.balance.demand !== 0n);
  // Every row's breakdown and its spoken text, built once per hour (and reply, names
  // and window); moving the pointer reuses them.
  const described = useMemo(() => new Map(data.resources.map(resource => {
    const content = breakdown(resource, hour);
    return [resource.id, { content, text: tooltipText(content ?? { title: resourceName(resource), rows: [] }) }] as const;
  })), [data, breakdown, hour]);
  // Hover and focus follow playback: the row shown is re-described whenever the hour changes (the kit's row tooltips).
  const content = useCallback((id: number) => described.get(id)?.content ?? null, [described]);
  const rowTips = useRowTooltips<number>({ owner, active, content, revision: `${hour}:${data.branchId}`, onMenu: (_, x, y) => onMenu(hour, x, y) });
  // Only the shown resource's fitted rows ask for partner names and flags.
  const tipHour = rowTips.hovered != null && active ? hour : null;
  const tipContent = rowTips.hovered != null ? described.get(rowTips.hovered)?.content : null;
  useEffect(() => onPartners(tipHour, tipContent), [tipHour, tipContent, onPartners]);
  useEffect(() => () => onPartners(null), [onPartners]);
  const any = (pick: (balance: Balance) => bigint) => rows.some(row => row.balance && pick(row.balance) !== 0n);
  const [supplyColor, demandColor] = [tokens.categories[0], tokens.categories[1]];
  const legend = [
    { id: 'produced', label: 'Produced', color: supplyColor },
    ...(any(balance => balance.fromSubjects) ? [{ id: 'subjects', label: 'From subjects', color: `color-mix(in srgb, ${supplyColor} 75%, ${tokens.surface})` }] : []),
    { id: 'imported', label: 'Imported', color: `color-mix(in srgb, ${supplyColor} 50%, ${tokens.surface})` },
    { id: 'needed', label: 'Needed', color: demandColor },
    ...(any(balance => balance.toOverlord) ? [{ id: 'overlord', label: 'To the overlord', color: `color-mix(in srgb, ${demandColor} 75%, ${tokens.surface})` }] : []),
    { id: 'exported', label: 'Exported', color: `color-mix(in srgb, ${demandColor} 50%, ${tokens.surface})` },
  ];
  const colour = new Map(legend.map(item => [item.id, item.color]));
  // Bars glide over the app's motion time; numbers are exact at once.
  const glide = { '--resource-glide': `${Math.round(3 * easeTau(tokens.motion, hoursPerSecond, tokens.reducedMotion))}ms` } as CSSProperties;
  const width = (raw: bigint) => largest > 0n ? `${Number(raw * 100000n / largest) / 1000}%` : '0%';
  // Every part stays mounted so it glides in and out; only a shown part is set apart by a hairline.
  const segments = (parts: readonly [string, bigint][]) => parts.map(([id, raw], index) =>
    <i key={id} style={{ width: width(raw), background: colour.get(id), marginLeft: index > 0 && raw > 0n ? 1 : 0 }} />);
  const describe = (resource: ResourceInfo, balance: Balance | null) => balance
    ? `${resourceName(resource)}: supply ${formatAmount(balance.supply, data.decimals)}, demand ${formatAmount(balance.demand, data.decimals)}, ${balance.net < 0n ? 'deficit' : 'surplus'} ${formatSigned(balance.net, data.decimals)}`
    : `${resourceName(resource)}: not recorded at this time`;
  return <>
    <p className="resource-caption">Per day</p>
    <Legend label="Bar parts" items={legend} />
    <ol className="resource-bars" style={glide} aria-label={`Resources on ${dayLabel(hour)}`}>
      {rows.map(({ resource, balance, glides }) => <li key={resource.id} role="button" aria-label={describe(resource, balance)} {...rowTips.rowProps(resource.id)}
        onClick={() => onPick(resource, hour)} onKeyDown={event => {
          if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onPick(resource, hour); }
          else rowTips.rowProps(resource.id).onKeyDown(event);
        }}
        // Across a gap or a change of country the bar jumps, as a line breaks there.
        style={glides ? undefined : { '--resource-glide': '0ms' } as CSSProperties}>
        {/* The breakdown the focus tooltip shows, in full even where the window trims it (the tooltip layer is not read out). */}
        <span id={rowTips.descriptionId(resource.id)} className="sr-only">{described.get(resource.id)?.text}</span>
        <span className="resource-name">{resource.icon ? <img src={resource.icon} alt="" /> : null}
          <span>{resourceName(resource)}</span></span>
        {balance ? <>
          <span className="resource-track" aria-hidden="true">
            <span className="resource-bar">{segments([['produced', balance.produced], ['subjects', balance.fromSubjects], ['imported', balance.imported]])}</span>
            <span className="resource-bar">{segments([['needed', balance.projects + balance.production], ['overlord', balance.toOverlord], ['exported', balance.exportReserve]])}</span>
          </span>
          <b className={balance.net < 0n ? 'is-negative' : 'is-positive'}>{formatSigned(balance.net, data.decimals)}</b>
        </> : <span className="resource-missing">Not recorded</span>}
      </li>)}
    </ol>
  </>;
}

/** Supply (solid) and demand (dashed) over time for the chosen resources, each in
 * its own palette colour; with one resource the gap between them is shaded. */
function OverTime({ data, numbers, within, tokens, viewport, options, runtime, track, extent, playhead, active, partnerAt, partnersRevision, onPartners, onMenu, setOption, onPick }: {
  data: ResourceBalanceReply; tokens: ChartTokens; options: Record<string, unknown>; runtime: WidgetProps['runtime']; track: BoundTrack;
  numbers: ResourceBalanceReply | undefined; within?: ComparisonProps['within'];
  extent: [number, number]; playhead: number | null; active: boolean; partnerAt: (hour: number) => CountryLabel; partnersRevision: number;
  onPartners: PartnerTargets; onMenu: (hour: number, x: number, y: number) => void; setOption: WidgetProps['setOption']; viewport: number;
  onPick: (resource: ResourceInfo, hour: number) => void;
}) {
  const storedText = typeof options.resources === 'string' ? options.resources : '';
  const stored = useMemo(() => contentSet(storedText), [storedText]);
  const chosen = useMemo(() => chosenResources(data, stored), [data, stored]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  // Each line keeps its colour from the game's order where it is free; lines past the
  // palette (a repeated token's extra lines) take other dashes, so no two look alike.
  const chosenInfo = useMemo(() => chosenLines(data, chosen), [data, chosen]);
  const pickScope = JSON.stringify([track.source.id, track.source.revision, track.branch.id, data.tag, chosenInfo.map(item => item.id)]);
  const pickOwner = useMemo(() => ({ scope: pickScope, numbers, within }), [pickScope, numbers, within]);
  const [selection, setSelection] = useState<{ hour: number; ids: readonly string[] } | null>(null);
  useEffect(() => setSelection(null), [pickOwner, extent[0], extent[1]]);
  const styles = useMemo(() => lineStyles(chosenInfo.map(item => ({ key: item.id, preferred: data.resources.indexOf(item) })), tokens.categories), [data, chosenInfo, tokens]);
  // Only the hours shown (a date range, or the branch): an axis dragged past a range
  // draws, shades, fits and describes nothing beyond it.
  const lines = useMemo(() => chosenInfo.flatMap(resource => {
    const recorded = data.series.find(item => item.id === resource.id);
    const series = recorded && { id: recorded.id, runs: clipToSpans(recorded.runs, [{ firstHour: extent[0], lastHour: extent[1] }]) };
    const style = styles.get(resource.id)!;
    return series ? [{ resource, series, color: style.color, dashes: style, ...supplyDemandRuns(series, data.decimals) }] : [];
  }), [chosenInfo, data, styles, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  // The keyboard steps where a chosen resource's values or deals change; the kit adds
  // the start of a range or zoom that begins between two, so the value going on into
  // it is reachable too.
  const keyHours = useMemo(() => changeHours(data, chosenInfo.map(item => item.id)), [data, chosenInfo]);
  useEffect(() => () => onPartners(null), [onPartners]);
  const shade = lines.length === 1 && options.gap !== false;
  const layers = useMemo<ChartLayer[]>(() => {
    const list: ChartLayer[] = [];
    if (shade) {
      // Bounded by the very curves the two lines are drawn with, green where supply is higher.
      const line = lines[0];
      list.push({ draw: (ctx, frame, chartTokens, state) => {
        drawGapFill(ctx, frame, line.supply, line.demand, chartTokens.positive, chartTokens.error, { until: state.playhead ?? undefined });
      } });
    }
    for (const line of lines) {
      list.push({
        draw: (ctx, frame, chartTokens, state) => {
          drawLine(ctx, frame, chartTokens, line.supply, line.color, { until: state.playhead ?? undefined, dash: line.dashes.primary.length ? line.dashes.primary : undefined });
          drawLine(ctx, frame, chartTokens, line.demand, line.color, { until: state.playhead ?? undefined, dash: line.dashes.secondary, endDot: false });
        },
        extent: x => extentOfAll([line.supply, line.demand], x[0], Math.min(x[1], until ?? x[1])),
      });
    }
    return list;
  }, [lines, shade, data.decimals, until]);
  // Only the clicked resource changes: saved resources this campaign lacks are kept,
  // and one this campaign shows always stays chosen. A chip whose
  // click cannot act is disabled and says why, pressed or not.
  // Each selected declaration keeps its name, value and two curve styles.
  // Toggling any declaration still selects or clears its whole token.
  const chips = data.resources.filter(item => item.ordinal === 1 || chosen.includes(item.token));
  const actions = new Map(chips.map(item => [item.token, toggleChosen(data, stored, chosen, item.token, Number.MAX_SAFE_INTEGER)]));
  const pick = (token: string) => {
    const action = actions.get(token);
    if (action && 'value' in action && setOption) setOption('resources', action.value);
  };
  const head = Math.floor(playhead ?? extent[1]);
  return <>
    <div className="chip-set" role="group" aria-label="Resources">{chips.map(item => {
      const action = actions.get(item.token);
      const reason = !setOption ? 'Resource selection unavailable.' : action && 'reason' in action ? action.reason : undefined;
      const selected = chosen.includes(item.token);
      const line = lines.find(line => line.resource.id === item.id);
      const run = line && runAt(line.series.runs, head);
      const value = run ? formatSigned(balanceOf(run).net, data.decimals) : 'Not recorded';
      const style = styles.get(item.id);
      return <button key={item.id} type="button" className="chip" aria-pressed={selected} aria-disabled={reason ? true : undefined}
        aria-label={`${resourceName(item)}${selected ? `, net ${value} per day` : ''}`}
        data-tooltip={reason} onClick={() => { if (!reason && setOption) pick(item.token); }}>
        {selected && style && <svg className="resource-series-key" width="28" height="14" aria-hidden="true">
          <line x1="0" y1="3" x2="28" y2="3" stroke={style.color} strokeWidth="2" strokeDasharray={style.primary.join(' ')} />
          <line x1="0" y1="11" x2="28" y2="11" stroke={style.color} strokeWidth="2" strokeDasharray={style.secondary.join(' ')} />
        </svg>}
        {item.icon && <img src={item.icon} alt="" />}{resourceName(item)}
        {selected && <span className="resource-chip-value">{value}</span>}
      </button>;
    })}</div>
    <p className="chart-line-key">Supply above demand in each key · per day</p>
    {selection && <div className="select-row">Choose a drawn resource on {dayLabel(selection.hour)} <Select autoFocus label="Drawn resource" value="" onChange={id => {
      const resource = chosenInfo.find(resource => String(resource.id) === id);
      if (resource && selection.ids.includes(String(resource.id))) { onPick(resource, selection.hour); setSelection(null); }
    }} options={[{ value: '', label: 'Choose resource…', disabled: true },
      ...chosenInfo.filter(resource => selection.ids.includes(String(resource.id))).map(resource => ({ value: String(resource.id), label: resourceName(resource) }))]} /></div>}
    {/* The tooltip closes when the pointer leaves: its partners' names and flags are released. */}
    <div className="resource-chart" onPointerLeave={() => onPartners(null)}>
      <TimeChart label={`Resource supply and demand: ${lines.map(line => resourceName(line.resource)).join(', ')}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        layers={layers} active={active} revision={`${head}:${data.branchId}:${chosen.join(' ')}:${shade}:${partnersRevision}:${viewport}`}
        keyHours={keyHours} pickOwner={pickOwner} onPick={(hour, context) => {
          const ids = pickLines(lines.flatMap(line => [line.supply, line.demand].map(runs => ({ key: String(line.resource.id), runs }))), hour, context);
          if (ids.length === 1) {
            const resource = chosenInfo.find(resource => String(resource.id) === ids[0]);
            if (resource) { setSelection(null); onPick(resource, hour); }
          } else setSelection(ids.length ? { hour, ids } : null);
        }}
        tooltip={hour => {
          if (hour < extent[0] || hour > extent[1]) { onPartners(null); return null; }
          const partner = partnerAt(hour);
          const known = dealsKnown(data, hour);
          const content = overTimeTooltip(`${dayLabel(hour)} · per day`, lines.map(line => {
            const run = runAt(line.series.runs, hour);
            return { resource: line.resource, color: line.color, balance: run ? balanceOf(run) : null, deals: run && known ? tradesAt(tradeIndex(data), line.resource.id, hour) : null };
          }), data.decimals, partner, viewport);
          onPartners(hour, content);
          return content;
        }}
        onContextMenu={(hour, x, y) => onMenu(hour, x, y)} />
    </div>
  </>;
}
