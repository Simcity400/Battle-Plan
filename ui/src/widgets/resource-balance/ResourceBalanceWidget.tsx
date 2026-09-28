import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from 'react';
import { ChipSet } from '../../components/ChipSet';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel, drawGapFill, drawLine } from '../../charts/draw';
import { easeTau } from '../../charts/easing';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { lineStyles } from '../../charts/palette';
import { clipToSpans, extentOfAll, runAt, sameStretch } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import { hideTooltip, showTooltip, tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount, formatSigned } from '../../charts/valueAxis';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { NameAt } from '../../dashboard/countryNamesAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import { contentSet, resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { readAt, readNote } from '../../dashboard/reads';
import type { ResourceBalanceReply, ResourceInfo } from '../../generated/dashboard';
import { balanceOf, barScale, chosenLines, chosenResources, dealsKnown, FULL_BREAKDOWN_MAX, orderResources, overTimeTooltip, partnersAt, resourceName,
  resourceTooltip, supplyDemandRuns, toggleChosen, tradeIndex, tradesAt } from './balanceData';
import type { Balance, RowOrder } from './balanceData';
import { MAX_CHOSEN, OPTIONS } from './definition';
import { useResourceBalance } from './useResourceBalance';
import './resourceBalance.css';

const NO_WARNINGS: string[] = [];

/** How often resources are read, for the note under a value read earlier. */
const CADENCE = 'resources are read once a day.';

/** Trade partners named and flagged as the recording showed them at the read behind
 * the tooltip (`useCountryLabelsAt`): only the read a tooltip shows is asked for. */
function usePartnersAt(runtime: WidgetProps['runtime'], track: BoundTrack | undefined, data: ResourceBalanceReply | undefined, read: number | null, active: boolean) {
  const wanted = useMemo<NameAt[]>(() => data && read != null ? partnersAt(tradeIndex(data), read).map(tag => ({ hour: read, tag })) : [], [data, read]);
  const { labelAt, revision } = useCountryLabelsAt(runtime.client, track, wanted, active);
  return { partnerAt: labelAt, revision };
}

/** One country's resources: per resource, supply (produced, then imported) against
 * demand (needed, then exported) with the game's surplus or deficit, or supply and
 * demand over time. The hover lists each trade partner. */
export default function ResourceBalanceWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data, error, retry } = useResourceBalance(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = Math.floor(playhead ?? extent[1]);
  // The read behind the tooltip being shown, whose partners are named.
  const [tipRead, setTipRead] = useState<number | null>(null);
  const partners = usePartnersAt(runtime, track, data, tipRead, active);
  const menu = useContextMenu('Resource actions');
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  // A widget out of view shows no tooltip, so it needs no partner names either.
  useEffect(() => { if (!active) setTipRead(null); }, [active]);
  // One resource's breakdown at an hour. It changes only with the reply, the partners'
  // names and the window, so the breakdowns built for an hour are reused while the
  // pointer moves; the deals come from the reply's index, never a scan.
  const breakdown = useCallback((resource: ResourceInfo, hour: number): TooltipContent | null => {
    if (!data) return null;
    const series = data.series.find(item => item.id === resource.id);
    const run = series && runAt(series.runs, hour);
    if (!run) return { title: resourceName(resource), icon: resource.icon ?? undefined, subtitle: dayLabel(hour), rows: [{ label: 'Not recorded at this time', tone: 'muted' }] };
    // A run starts at a read, so the read behind it is never missing.
    const read = readAt(data.reads, hour) ?? run.firstHour;
    // As much as fits the window, with the surplus or deficit always shown.
    return resourceTooltip({ title: resourceName(resource), icon: resource.icon ?? undefined, subtitle: `Per day · ${dayLabel(hour)}`, note: readNote(read, hour, CADENCE) },
      balanceOf(run), dealsKnown(data, hour) ? tradesAt(tradeIndex(data), resource.id, hour) : null, data.decimals, partners.partnerAt(read), viewport);
  }, [data, partners.partnerAt, viewport]);

  if (!track || !tag) return <WidgetState kind="empty" icon="layers" message="Load a campaign and choose a country to see its resources." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the country’s recorded branch." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading resources…" />;
  if (!data.reads.length || !data.resources.length) return <WidgetState kind="unavailable" message="No resources were recorded for this country in this branch." />;

  const jump = (hour: number) => [jumpItem(runtime, track.id, hour)];

  return <div className="resource-balance">
    {options.view === 'time'
      ? <OverTime data={data} tokens={tokens} viewport={viewport} options={options} runtime={runtime} track={track} extent={extent} playhead={playhead} active={active}
        partnerAt={partners.partnerAt} partnersRevision={partners.revision} onRead={setTipRead}
        onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} setOption={setOption} />
      : <BalanceBars data={data} tokens={tokens} hour={head} order={options.order as RowOrder} scale={options.scale === 'moment' ? 'moment' : 'campaign'}
        hideEmpty={options.hideEmpty === true} owner={`resource-balance:${widget.id}`} breakdown={breakdown} onRead={setTipRead} active={active}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} />}
    {menu.menu}
  </div>;
}

/** Per resource, a supply bar above a demand bar on a shared scale, with the net at
 * the right (mock-up economy-1). Rows are focusable: hover or focus shows the
 * breakdown, right-click or the context-menu key jumps to the read shown. */
function BalanceBars({ data, tokens, hour, order, scale, hideEmpty, owner, breakdown, onRead, active, hoursPerSecond, onMenu }: {
  data: ResourceBalanceReply; tokens: ChartTokens; hour: number; order: RowOrder; scale: 'campaign' | 'moment'; hideEmpty: boolean; owner: string;
  breakdown: (resource: ResourceInfo, hour: number) => TooltipContent | null; onRead: (read: number | null) => void; active: boolean;
  hoursPerSecond: number; onMenu: (hour: number, x: number, y: number) => void;
}) {
  const campaignScale = useMemo(() => barScale(data.series, null), [data]);
  const largest = scale === 'moment' ? barScale(data.series, hour) : campaignScale;
  const read = readAt(data.reads, hour);
  // The hour drawn last, so a bar glides only from a value of the same stretch of recording.
  const drawnHour = useRef(hour);
  useEffect(() => { drawnHour.current = hour; }, [hour]);
  const rows = orderResources(data, hour, order).map(resource => {
    const series = data.series.find(item => item.id === resource.id);
    const run = series ? runAt(series.runs, hour) : null;
    return { resource, balance: run ? balanceOf(run) : null, glides: series != null && sameStretch(series.runs, drawnHour.current, hour) };
  }).filter(row => !hideEmpty || !row.balance || row.balance.supply > 0n || row.balance.demand > 0n);
  // Hover follows playback: the pointer's row is re-described whenever the hour changes.
  const [hovered, setHovered] = useState<{ id: number; x: number; y: number } | null>(null);
  // Every row's breakdown and its spoken text, built once per hour (and reply, names
  // and window); moving the pointer reuses them.
  const described = useMemo(() => new Map(data.resources.map(resource => {
    const content = breakdown(resource, hour);
    return [resource.id, { content, text: tooltipText(content ?? { title: resourceName(resource), rows: [] }) }] as const;
  })), [data, breakdown, hour]);
  // Out of view (scrolled away, a hidden tab), the widget shows no tooltip and forgets the pointer.
  useEffect(() => { if (!active) { setHovered(null); hideTooltip(owner); } }, [active, owner]);
  // Only a shown tooltip's read has its partners named.
  const hoveredRead = hovered && active ? read : null;
  useEffect(() => onRead(hoveredRead), [hoveredRead, onRead]);
  useEffect(() => {
    const content = hovered && active ? described.get(hovered.id)?.content : null;
    if (hovered && content) showTooltip(owner, content, hovered.x, hovered.y, `${hovered.id}:${hour}:${data.branchId}`);
    else hideTooltip(owner);
  }, [hovered, hour, data, owner, described, active]);
  useEffect(() => () => hideTooltip(owner), [owner]);
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
  const descriptionId = (resource: ResourceInfo) => `${owner}-${resource.id}-breakdown`;
  const pointAt = (element: HTMLElement) => { const rect = element.getBoundingClientRect(); return { x: rect.right - 8, y: rect.top + rect.height / 2 }; };
  const jumpHour = read ?? hour;
  return <>
    <p className="resource-caption">Per day{read != null ? ` · read ${formatDateTime(read)}` : ''}</p>
    <Legend label="Bar parts" items={legend} />
    <ol className="resource-bars" style={glide} aria-label={`Resources on ${dayLabel(hour)}`}>
      {rows.map(({ resource, balance, glides }) => <li key={resource.id} tabIndex={0} aria-label={describe(resource, balance)} aria-describedby={descriptionId(resource)}
        // Across a gap or a change of country the bar jumps, as a line breaks there.
        style={glides ? undefined : { '--resource-glide': '0ms' } as CSSProperties}
        onPointerMove={(event: ReactPointerEvent) => setHovered({ id: resource.id, x: event.clientX, y: event.clientY })}
        onPointerLeave={() => setHovered(null)}
        onFocus={event => setHovered({ id: resource.id, ...pointAt(event.currentTarget) })}
        onBlur={() => setHovered(null)}
        onContextMenu={event => { event.preventDefault(); onMenu(jumpHour, event.clientX, event.clientY); }}
        onKeyDown={(event: ReactKeyboardEvent<HTMLLIElement>) => {
          if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
            event.preventDefault();
            const point = pointAt(event.currentTarget);
            onMenu(jumpHour, point.x, point.y);
          } else if (event.key === 'Escape') setHovered(null);
        }}>
        {/* The breakdown the focus tooltip shows, in full even where the window trims it (the tooltip layer is not read out). */}
        <span id={descriptionId(resource)} className="sr-only">{described.get(resource.id)?.text}</span>
        <span className="resource-name">{resource.icon ? <img src={resource.icon} alt="" /> : <span className="resource-icon-missing" aria-hidden="true" />}
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
function OverTime({ data, tokens, viewport, options, runtime, track, extent, playhead, active, partnerAt, partnersRevision, onRead, onMenu, setOption }: {
  data: ResourceBalanceReply; tokens: ChartTokens; options: Record<string, unknown>; runtime: WidgetProps['runtime']; track: BoundTrack;
  extent: [number, number]; playhead: number | null; active: boolean; partnerAt: (read: number) => CountryLabel; partnersRevision: number;
  onRead: (read: number | null) => void; onMenu: (hour: number, x: number, y: number) => void; setOption: WidgetProps['setOption']; viewport: number;
}) {
  const storedText = typeof options.resources === 'string' ? options.resources : '';
  const stored = useMemo(() => contentSet(storedText), [storedText]);
  const chosen = useMemo(() => chosenResources(data, stored), [data, stored]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  // Each line keeps its colour from the game's order where it is free; lines past the
  // palette (a repeated token's extra lines) take other dashes, so no two look alike.
  const chosenInfo = useMemo(() => chosenLines(data, chosen), [data, chosen]);
  const styles = useMemo(() => lineStyles(chosenInfo.map(item => ({ key: item.id, preferred: data.resources.indexOf(item) })), tokens.categories), [data, chosenInfo, tokens]);
  // Only the hours shown (a date range, or the branch): an axis dragged past a range
  // draws, shades, fits and describes nothing beyond it.
  const lines = useMemo(() => chosenInfo.flatMap(resource => {
    const recorded = data.series.find(item => item.id === resource.id);
    const series = recorded && { id: recorded.id, runs: clipToSpans(recorded.runs, [{ firstHour: extent[0], lastHour: extent[1] }]) };
    const style = styles.get(resource.id)!;
    return series ? [{ resource, series, color: style.color, dashes: style, ...supplyDemandRuns(series, data.decimals) }] : [];
  }), [chosenInfo, data, styles, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const overflow = lines.some(line => line.dashes.primary.length > 0);
  useEffect(() => () => onRead(null), [onRead]);
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
  // and one this campaign shows always stays chosen, as in the mock-up. A chip whose
  // click cannot act is disabled and says why, pressed or not.
  // One chip per token: a repeated declaration is chosen with its token.
  const chips = data.resources.filter(item => item.ordinal === 1);
  const actions = new Map(chips.map(item => [item.token, toggleChosen(data, stored, chosen, item.token, MAX_CHOSEN)]));
  const pick = (token: string) => {
    const action = actions.get(token);
    if (action && 'value' in action && setOption) setOption('resources', action.value);
  };
  const head = Math.floor(playhead ?? extent[1]);
  return <>
    {setOption && <ChipSet label="Resources" chips={chips.map(item => {
      const action = actions.get(item.token);
      return { value: item.token, label: resourceName(item), icon: item.icon, unavailable: action && 'reason' in action ? action.reason : undefined };
    })} selected={chosen} max={MAX_CHOSEN} onToggle={pick} />}
    <Legend label="Resources" items={lines.map(line => {
      const run = runAt(line.series.runs, head);
      return { id: String(line.resource.id), label: resourceName(line.resource), color: line.color,
        value: run ? formatSigned(balanceOf(run).net, data.decimals) : 'Not recorded' };
    })} />
    <p className="resource-key"><i aria-hidden="true" />Solid: supply · <i className="is-dashed" aria-hidden="true" />dashed: demand, per day
      {overflow && ` · every further ${tokens.categories.length} lines repeat the colours with their own dashes`}</p>
    {/* The tooltip closes when the pointer leaves: its partners' names and flags are released. */}
    <div className="resource-chart" onPointerLeave={() => onRead(null)}>
      <TimeChart label={`Resource supply and demand: ${lines.map(line => resourceName(line.resource)).join(', ')}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        layers={layers} active={active} revision={`${head}:${data.branchId}:${chosen.join(' ')}:${shade}:${partnersRevision}:${viewport}`}
        // The keyboard steps through the reads; the kit adds the start of a range or zoom
        // that begins between two reads, so the value carried into it is reachable too.
        keyHours={data.reads}
        tooltip={hour => {
          if (hour < extent[0] || hour > extent[1]) { onRead(null); return null; }
          const read = readAt(data.reads, hour);
          // Partners are named for the read behind this hour while breakdowns can list them:
          // shown when they fit, and read out in full with the keyboard either way.
          onRead(lines.length <= FULL_BREAKDOWN_MAX ? read : null);
          const partner = read != null ? partnerAt(read) : (tag: string) => ({ name: tag });
          const known = dealsKnown(data, hour);
          return overTimeTooltip(`${dayLabel(hour)} · per day`, lines.map(line => {
            const run = runAt(line.series.runs, hour);
            return { resource: line.resource, color: line.color, balance: run ? balanceOf(run) : null, deals: run && known ? tradesAt(tradeIndex(data), line.resource.id, hour) : null };
          }), data.decimals, partner, readNote(read, hour, CADENCE), viewport);
        }}
        onContextMenu={(hour, x, y) => onMenu(hour, x, y)} />
    </div>
  </>;
}
