import { SPEC } from '../../dashboard/regionTime';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { useCallback, useId, useMemo,  useRef, useState } from 'react';
import { setStyle, setText } from '../../charts/domText';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel, drawEndLabels } from '../../charts/draw';
import { drawLineSummary, summaryExtent } from '../../charts/chartSummary';
import { jumpItem } from '../../charts/jump';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip, tooltipText } from '../../charts/tooltip';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { runAt } from '../../charts/series';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import Legend from '../../charts/Legend';
import { motionOf } from '../../charts/easing.ts';
import CountryComparisons from '../../dashboard/CountryComparisons';
import type { ComparisonProps } from '../../dashboard/CountryComparisons';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { keyId } from '../../dashboard/keyedSeriesData';
import { useKeyedSeries, useSeriesParts } from '../../dashboard/keyedSeries';
import { usePlaybackHour } from '../../dashboard/follow/useFollow';
import { CONSUMPTION, OPTIONS } from './definition';
import { FUEL_BRANCHES, FUEL_CONSUMERS, barWidths, branchName, branchesAt, fuelAmount, fuelDecimals, fuelStructure, percentText, ratioAxis,
  validateFuelBranches } from './fuelBranchesData';
import type { BranchAt } from './fuelBranchesData';
import './fuelBranches.css';
import type { FuelBranchesReply } from '../../generated/dashboard';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import { UNRECORDED_MESSAGE } from '../../dashboard/regionTime.ts';

const NO_WARNINGS: readonly string[] = [];
const UNNAMED = 'Country name unavailable';
const SHARE = { kind: FUEL_CONSUMERS, measure: 'receivedShare' } as const;
const SHARE_PLACES = 4;

export default function FuelBranchesWidget(props: WidgetProps) {
  return <CountryComparisons {...props} component={FuelBranchesCountry} />;
}

const isMember = (within: ComparisonProps['within'], hour: number | null) =>
  !within || hour === null || within.some(span => span.firstHour <= hour && hour <= span.lastHour);

function FuelBranchesCountry(props: ComparisonProps) {
  const { widget, runtime } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  if (!track) return <WidgetState kind="empty" icon="droplet" message="Load a campaign to see fuel." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  return options.view === 'time' ? <FuelBranchesOverTime {...props} tag={tag} /> : <FuelBranchBars {...props} tag={tag} />;
}

interface RowCells { received?: HTMLElement | null; requested?: HTMLElement | null; unmet?: HTMLElement | null; delivered?: HTMLElement | null; short?: HTMLElement | null }

function FuelBranchBars({ widget, runtime, active, onDiagnostics, within, tag }: ComparisonProps & { tag: string }) {
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const menu = useContextMenu('Fuel by branch actions');
  const owner = useId();
  const digits = fuelDecimals(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0])!;
  const slots = useSlotTags(runtime.client, track, active && isSlotKey(tag));
  const cells = useRef(new Map<number, RowCells>());
  const paint = useCallback((reply: FuelBranchesReply) => {
    const rows = branchesAt(reply), widths = barWidths(rows);
    for (const row of rows) {
      const held = cells.current.get(row.index), width = widths.get(row.index);
      if (!held || !width) continue;
      setText(held.received, fuelAmount(row.received, digits));
      setText(held.requested, ` / ${fuelAmount(row.requested, digits)}`);
      setText(held.unmet, ` ${fuelAmount(row.unmet, digits)}`);
      setStyle(held.delivered, 'width', width.delivered);
      setStyle(held.short, 'width', width.unmet);
    }
  }, [digits]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), FUEL_BRANCHES, { country: countryFromKey(tag) }, { active, validate: validateFuelBranches, structure: fuelStructure, paint });
  const data = head.data;
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  const rows = useMemo(() => data ? branchesAt(data) : [], [data]);
  const widths = useMemo(() => barWidths(rows), [rows]);
  const hour = head.hour ?? data?.readHour ?? null;
  const tooltip = useCallback((row: BranchAt): TooltipContent => {
    const lines: TooltipRow[] = [
      { label: 'Requested', value: fuelAmount(row.requested, digits) },
      { label: 'Received', value: fuelAmount(row.received, digits) },
    ];
    if (row.unmet > 0n) lines.push({ label: 'Unmet', value: fuelAmount(row.unmet, digits) });
    lines.push({ label: 'Priority', value: String(row.priority) });
    if (data?.readHour != null) lines.push({ label: 'As of', value: dayLabel(data.readHour), tone: 'muted' });
    if (row.requested === 0n) lines.push({ label: 'No fuel requested', tone: 'muted' });
    return { title: row.name, rows: lines };
  }, [digits, data?.readHour]);
  const anchor = useRef<HTMLDivElement>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const drillRegion = drillControl.region;
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);
  const open = () => {
    const at = head.at ?? head.hour;
    if (at === null) return;
    dispatch({ type: 'open', breakdown: CONSUMPTION.id, hour: at, follow: drillControl.followsAt(at) });
  };
  const rowTips = useRowTooltips<number>({ owner, active, revision: `${data?.readHour}:${digits}:${fuelStructure(data ?? EMPTY)}`,
    content: index => { const row = rows.find(item => item.index === index); return row ? fitTooltip([tooltip(row)], viewport) : null; },
    onMenu: (_, x, y) => { if (data?.readHour != null) menu.open(x, y, [jumpItem(runtime, track.id, data.readHour)]); } });

  let body;
  if (head.error) body = <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  else if (head.read === 'off' && !data) body = <WidgetState kind="outside" />;
  else if (!data) body = <WidgetState kind="loading" label="Loading fuel by branch…" />;
  else if (!isMember(within, hour)) body = <WidgetState kind="unavailable" message="This country is not in the group at this date." />;
  else if (data.missing) body = <WidgetState kind="unavailable" message={data.missing} />;
  else if (!rows.length) body = <WidgetState kind="empty" icon="droplet" message={`${keyNameAt(slots, tag, hour ?? track.branch.lastHour) ?? UNNAMED} requested no fuel at this date.`} />;
  else body = <div className="resource-balance fuel-branches">
    <ol className="resource-bars" aria-label={hour === null ? 'Fuel by branch' : `Fuel by branch on ${dayLabel(hour)}`}>
      {rows.map(row => {
        const width = widths.get(row.index)!;
        const props = rowTips.rowProps(row.index);
        const content = tooltip(row);
        const held = (cell: keyof RowCells) => (element: HTMLElement | null) => {
          const entry = cells.current.get(row.index) ?? {};
          entry[cell] = element;
          cells.current.set(row.index, entry);
        };
        return <li key={row.index} {...props} role="button" onClick={open} onKeyDown={event => {
          if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); open(); } else props.onKeyDown(event);
        }}>
          <span id={rowTips.descriptionId(row.index)} className="sr-only">{tooltipText(content)}</span>
          <span className="resource-name"><span>{row.name}</span></span>
          {row.requested === 0n ? <><span className="resource-track" aria-hidden="true" /><b data-tooltip="No fuel requested">—</b></>
            : <>
              <span className="resource-track fuel-branch-track" aria-hidden="true">
                <span className="resource-bar">
                  <i ref={held('delivered')} style={{ width: width.delivered, background: tokens.categories[0] }} />
                  {row.unmet > 0n && <i ref={held('short')} style={{ width: width.unmet, background: tokens.error, marginLeft: 1 }} />}
                </span>
                {row.unmet > 0n && <small className="fuel-branch-unmet">unmet<span ref={held('unmet')}> {fuelAmount(row.unmet, digits)}</span></small>}
              </span>
              <b><span ref={held('received')} className="fuel-branch-received">{fuelAmount(row.received, digits)}</span>
                <span ref={held('requested')} className="fuel-branch-requested"> / {fuelAmount(row.requested, digits)}</span></b>
            </>}
        </li>;
      })}
    </ol>
    {menu.menu}
  </div>;
  const at = drillAt;
  return <div ref={anchor} className="fuel-branches-host">
    <DrillHost drill={drillControl} runtime={runtime} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor} title={CONSUMPTION.label}
      day={at === null ? '' : dayLabel(at)} canFollow onClose={() => dispatch({ type: 'close' })} main={<div ref={head.root} className="fuel-branches-host">{body}</div>}>
      <FuelConsumptionDrill />
    </DrillHost>
  </div>;
}

function FuelConsumptionDrill() {
  return <WidgetState kind="unavailable" message="Current consumption comes with the Fuel widget, which is not back in this build yet." />;
}

const EMPTY: FuelBranchesReply = { tag: '', scale: 100000, consumerNames: [], readHour: null, consumers: [], missing: null, warnings: [] };

function FuelBranchesOverTime({ widget, runtime, active, onDiagnostics, within, tag }: ComparisonProps & { tag: string }) {
  const options = resolveOptions(OPTIONS, widget.options);
  const viewport = useViewportHeight();
  const menu = useContextMenu('Fuel by branch actions');
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0])!;
  const slots = useSlotTags(runtime.client, track, active && isSlotKey(tag));
  const following = widget.time.mode === 'follow';
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const playbackHour = usePlaybackHour(runtime.clock, active && following);
  const window = resolveWindow(widget, track, runtime.tracks, playbackHour);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour];
  const axisOffset = track.axisOffset ?? 0;
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const gate = useGate(runtime, widget, SPEC, active);
  const span = gate.span(extent[0], extent[1]);
  const parts = useSeriesParts(runtime.client, gate, track.source, track.branch.id, SHARE, useMemo(() => ({ key: tag, spans: within ?? null }), [tag, within]), span, active);
  const namesAt = gate.recorded(extent[1]);
  const names = useProduct<FuelBranchesReply>(runtime.client, namesAt === null ? null : productRequest({ sourceId: track.source.id, revision: track.source.revision, product: FUEL_BRANCHES,
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour: namesAt } }), active, validateFuelBranches, track.source);
  const consumerNames = names.data?.consumerNames ?? [];
  const keys = useMemo(() => (parts.data?.parts ?? []).map(part => ({ tag, part: part.part })).sort((a, b) => Number(a.part) - Number(b.part)), [parts.data, tag]);
  const membership = useMemo(() => within ? new Map(keys.map(key => [keyId(key), within])) : undefined, [keys, within]);
  const series = useKeyedSeries(runtime.client, runtime.clock, gate, keys.length ? { source: track.source, branch: track.branch, family: SHARE, keys,
    partBits: parts.data?.partBits ?? 32, within: membership, lastHour: track.branch.lastHour } : null,
  following ? null : gate.hour, gate.recorded(hoverHour, within), following, active, root);
  useDiagnostics(onDiagnostics, names.data?.warnings ?? NO_WARNINGS);
  const lines = useMemo(() => keys.map((key, index) => ({ key, color: tokens.categories[index % tokens.categories.length],
    name: branchName(consumerNames, Number(key.part)) })), [keys, tokens.categories, consumerNames]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const most = useMemo(() => {
    let top = 0;
    for (const line of lines) {
      const shown = series.summaryByKey.get(keyId(line.key))?.summary;
      const range = shown ? summaryExtent(shown, extent, until) : null;
      if (range && range[1] > top) top = range[1];
    }
    return top;
  }, [lines, series.summaryByKey, extent, until]);
  const axis = useMemo(() => ratioAxis(most), [most]);
  const ends = options.endLabels !== false;
  const reachedByKey = useMemo(() => new Map(series.lines.map(line => [keyId(line.key), line])), [series.lines]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const labels = lines.flatMap(line => {
        const shown = series.summaryByKey.get(keyId(line.key))?.summary;
        const end = shown ? drawLineSummary(ctx, frame, chartTokens, shown, line.color, true, state.playhead) : null;
        const run = state.playhead === null ? null : runAt(reachedByKey.get(keyId(line.key))?.runs ?? [], Math.floor(state.playhead));
        return ends && end && run ? [{ text: percentText(Number(run.valueRaw) / 10 ** SHARE_PLACES), y: end.y, from: end.y, color: line.color }] : [];
      });
      drawEndLabels(ctx, frame, chartTokens, labels);
    },
  }], [lines, series.summaryByKey, reachedByKey, ends]);

  const shownTag = keyNameAt(slots, tag, playhead ?? extent[1]);
  if (parts.error) return <WidgetState kind="error" failure={parts.error} onRetry={parts.retry} />;
  if (!parts.data) return <WidgetState kind="loading" label="Loading fuel by branch…" />;
  if (!keys.length) return parts.data.missing.length
    ? <WidgetState kind="unavailable" message={`${shownTag ?? UNNAMED} has no recorded fuel consumers here: ${parts.data.missing[0].reason}`} />
    : <WidgetState kind="empty" icon="droplet" message={`${shownTag ?? UNNAMED} requested no fuel in this time.`} />;
  if (series.error && !series.summaryByKey.size) return <WidgetState kind="error" failure={series.error} onRetry={series.retry} />;
  const tooltip = (at: number) => {
    if (!isMember(within, at)) return fitTooltip([{ title: dayLabel(at), rows: [{ label: 'Not in the group at this date', tone: 'muted' as const }] }], viewport);
    return fitTooltip([{ title: dayLabel(at), rows: lines.map(line => {
      const reached = reachedByKey.get(keyId(line.key));
      const run = runAt(reached?.runs ?? [], at);
      const missing = reached?.missing.get(at);
      const unrecorded = reached?.unrecorded.has(at) === true;
      return { label: line.name, strong: true, swatch: line.color,
        value: run ? percentText(Number(run.valueRaw) / 10 ** SHARE_PLACES) : missing ? (unrecorded ? 'Not recorded' : 'No share') : gate.recorded(at, within) === null ? UNRECORDED_MESSAGE : 'Loading…',
        detail: run ? undefined : missing, tone: run ? undefined : 'muted' as const };
    }) }], viewport);
  };
  return <div className="fuel-branches-time" ref={setRoot}>
    <Legend label="Branches" items={lines.map(line => ({ id: line.key.part, label: line.name, color: line.color }))} />
    <div className="fuel-branches-chart">
      <TimeChart label={`Fuel received by branch of ${shownTag ?? UNNAMED}`} extent={extent} playhead={playhead} liveHead={liveHead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: 0, yMax: axis.top, log: false }} valueTicks={axis.ticks}
        formatValue={value => `${value}%`} padding={ends ? { right: 64 } : undefined}
        layers={layers} active={active} pickOwner={series.owner} notRecorded={series.unrecorded}
        onSummaryView={series.onSummaryView} onNavigateHour={series.onNavigateHour}
        keyboardHour={series.keyboardHour} keyboardRevision={series.keyboardRevision} onHover={setHoverHour}
        revision={`${track.branch.id}:${tag}:${lines.length}:${ends}:${series.states ? 'states' : 'loading'}`}
        tooltip={at => tooltip(Math.floor(at))}
        onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, at)])} />
    </div>
    {menu.menu}
  </div>;
}
