import { countryFromKey } from '../../dashboard/countryKey.ts';
import { failureDetails } from '../../dashboard/queryError.ts';
import { useCallback, useMemo,  useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import HeatChart from '../../charts/heat/HeatChart';
import type { HeatPeriod, HeatRow } from '../../charts/heat/layout';
import { hatchStyle } from '../../charts/hatch';
import { jumpItem } from '../../charts/jump';
import { dayLabel, drawLine, placeEndLabels } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import type { PlotFrame } from '../../charts/frame';
import LiveText, { liveTexts } from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer, LiveHead } from '../../charts/TimeChart';
import { extentOf, lineEndHour, runAt as plottedAt } from '../../charts/series';
import type { Run } from '../../charts/series';
import type { ChartTokens } from '../../charts/tokens';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import Legend from '../../charts/Legend';
import { motionOf } from '../../charts/easing.ts';
import type { WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { resolveWindow } from '../../dashboard/model';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { nameReadMetadata, nameReadFailure } from '../../dashboard/nameReads.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { perfLog } from '../../shell/perf.ts';
import { CONVOY_HEALTH, convoyRows, dayPeriods, factsOrReason, heatRows, labelDates, tableKnown, validateConvoys } from './convoyData';
import type { ConvoyRow } from './convoyData';
import { DETAILS, OPTIONS } from './definition';
import './convoyHealth.css';
import type { ConvoyHealthReply, ConvoyView } from '../../generated/dashboard';
import { regionFollows, SPEC } from '../../dashboard/regionTime.ts';
import { useGate } from '../../dashboard/regionGate';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: readonly string[] = [];
const VALIDATORS = { regions: validateConvoys('regions'), trade: validateConvoys('trade') };

function useLiveHead(runtime: WidgetRuntime, track: BoundTrack | undefined, following: boolean, widgetId: string): LiveHead | undefined {
  return useMemo(() => {
    if (!following || !track) return undefined;
    let shown: number | null = null;
    return {
      read: () => {
        const hour = runtime.clock.getSnapshot().position + track.axisOffset;
        const floor = Math.floor(hour);
        if (floor !== shown) { shown = floor; perfLog?.shown(widgetId, floor); }
        return hour;
      },
      subscribe: (changed: () => void) => { const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)]; return () => { for (const stop of stops) stop(); }; },
    };
  }, [following, track, runtime.clock, widgetId]);
}

export default function ConvoyHealthWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const menu = useContextMenu('Convoy health');
  const options = resolveOptions(OPTIONS, widget.options);
  const view: ConvoyView = options.view === 'trade' ? 'trade' : 'regions';
  const count = typeof options.rowCount === 'number' && options.rowCount > 0 ? options.rowCount : 10;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const following = widget.time.mode === 'follow';
  const window = track && resolveWindow(widget, track, runtime.tracks, following ? track.branch.lastHour - track.axisOffset : runtime.clock.getSnapshot().position);
  const pinnedHour = window && widget.time.mode === 'pin' ? Math.floor(window.lastHour) : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const liveHead = useLiveHead(runtime, track, following, widget.id);
  const playhead = following && track ? Math.floor(runtime.clock.getSnapshot().position + track.axisOffset) : pinnedHour;
  const gate = useGate(runtime, widget, SPEC, active);
  const span = gate.span(extent[0], extent[1]);
  const request = track && tag && span ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: CONVOY_HEALTH,
    params: { branchId: track.branch.id, view, country: countryFromKey(tag), from: span.firstHour, to: span.lastHour } }) : null;
  const { data, error, retry } = useProduct<ConvoyHealthReply>(runtime.client, request, active, VALIDATORS[view], track?.source);
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [picked, setPicked] = useState<string | null>(null);
  const wanted = useMemo(() => data ? labelDates(data) : [], [data]);
  const names = useCountryNamesAt(runtime.client, track, wanted, active, Math.max(1, new Set(wanted.map(item => item.hour)).size));
  const countryName = useCallback((country: string | null, at: number) => country === null ? nameReadMetadata(undefined)
    : nameReadMetadata(names.get(nameKey(at, country))), [names]);
  const periods = useMemo(() => track ? dayPeriods(extent, track.branch.gaps) : [], [track, extent[0], extent[1]]);
  const rows = useMemo(() => data && data.view === view ? convoyRows(data, countryName) : [], [data, view, countryName]);
  const color = tokens.categories[0];
  const heat = useMemo(() => heatRows(rows, periods, color, count, data?.scale ?? 1), [rows, periods, color, count, data?.scale]);
  const labels = useMemo(() => rows.map(row => row.label), [rows]);
  const byId = useMemo(() => new Map(rows.map(row => [row.id, row])), [rows]);
  const tooltip = useCallback((row: HeatRow, period: HeatPeriod | null, hour: number) => {
    if (!data) return null;
    const entry = byId.get(row.id);
    const { facts, reason } = factsOrReason(entry, period, data);
    const read = period?.readHour ?? null;
    return { detail: entry?.nameDetail, title: row.label, subtitle: dayLabel(period?.firstHour ?? hour), rows: [
      ...facts.map(fact => ({ label: fact.label, value: fact.value, ...(fact.detail ? { detail: fact.detail } : {}) })),
      ...reason ? [{ label: reason, tone: 'muted' as const }] : [],
      ...read !== null && facts.length ? [{ label: `As of ${dayLabel(read)}`, tone: 'muted' as const }] : [],
    ] };
  }, [byId, data]);
  const onMenu = useCallback((hour: number, x: number, y: number) => { if (track) menu.open(x, y, [jumpItem(runtime, track.id, hour)]); }, [track, menu, runtime]);
  const anchor = useRef<HTMLDivElement>(null);
  const onPick = useCallback((row: HeatRow, period: HeatPeriod | null, hour: number) => {
    setPicked(row.id);
    dispatch({ type: 'open', breakdown: DETAILS.id, hour: period?.readHour ?? hour, follow: drillControl.followsAt(period?.readHour ?? hour) });
  }, [runtime, track, following, pinnedHour]);
  const pickOwner = useMemo(() => ({ data, view, tag }), [data, view, tag]);
  const drilled = picked ? byId.get(picked) ?? null : null;
  const drillRegion = drillControl.region;
  const drillFollows = regionFollows(drillRegion) && drill.breakdown !== null;
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);
  const drillHour = drillFollows ? null : drillAt;
  const liveFacts = useMemo(() => drilled && drillFollows && liveHead
    ? liveTexts(runtime.clock, () => Math.floor(liveHead.read()), at => new Map([['day', dayLabel(at)], ...drilled.facts(at).map(fact => [fact.label, fact.value] as const)]))
    : null, [drilled, drillFollows, liveHead, runtime.clock]);

  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see convoys." />;
  if (!window) return <WidgetState kind="outside" />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data || data.view !== view) return <WidgetState kind="loading" label="Loading convoys…" />;
  if (!heat.length) {
    const shownHour = playhead ?? extent[1];
    const tables = view === 'regions' ? [0] : [1, 2];
    return tables.every(index => tableKnown(data, index, shownHour))
      ? <WidgetState kind="empty" icon="anchor" message="No convoy activity in this span." />
      : <WidgetState kind="unavailable" message="Convoys were not recorded in this span." />;
  }
  const legend = [{ id: 'full', label: view === 'regions' ? 'Convoy efficiency (0–100%)' : 'Trade efficiency (0–100%)', color }];
  const main = <div className="convoy-health">
    <div className="convoy-health-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: tokens.surface }} />{view === 'regions' ? 'No bad route' : 'No deal'}</span>
      <span><i aria-hidden="true" style={{ background: `color-mix(in srgb, ${tokens.error} ${tokens.gapOpacity * 100}%, ${tokens.surface})` }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(tokens)} />{view === 'trade' ? 'Efficiency not recorded' : 'Unknown'}</span>
    </div>
    <div className="convoy-health-chart"><HeatChart label={view === 'regions' ? 'Convoy efficiency by sea region' : 'Trade efficiency of imports'}
      rows={heat} periods={periods} labelCatalogue={labels} extent={extent} playhead={playhead} liveHead={liveHead} branchStart={track.branch.firstHour}
      legend={legend} afterPlayhead={!following} look="margin" active={active} hoursPerSecond={hoursPerSecond}
      revision={`${data.branchId}:${view}:${tag}:${heat.length}`}
      tooltip={tooltip} onContextMenu={onMenu} onPick={onPick} pickOwner={pickOwner} /></div>
    {menu.menu}
  </div>;
  return <div ref={anchor} className="convoy-health-host" data-detail={failureDetails([...names.values()].map(nameReadFailure))}>
    <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null && !!drilled} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={drilled?.label ?? DETAILS.label} day={liveFacts ? <LiveText live={liveFacts} at="day" /> : drillHour === null ? '' : dayLabel(drillHour)} canFollow
      onClose={() => dispatch({ type: 'close' })} main={main}>
      {drilled && <ConvoyDrill row={drilled} runtime={runtime} track={track} hour={drillHour} liveHead={drillFollows ? liveHead : undefined}
        liveFacts={liveFacts} extent={extent} active={active} scale={data.scale} onPick={hour => dispatch({ type: 'pick', hour })} />}
    </DrillHost>
  </div>;
}

function ConvoyDrill({ row, runtime, track, hour, liveHead, liveFacts, extent, active, scale, onPick }: { row: ConvoyRow; runtime: WidgetRuntime; track: BoundTrack;
  hour: number | null; liveHead: LiveHead | undefined; liveFacts: LiveTexts | null; extent: [number, number]; active: boolean; scale: number; onPick: (hour: number) => void }) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const percent = row.unit === 'percent';
  const lines = useMemo(() => row.lines.map((line, index) => ({ ...line, color: tokens.categories[index % tokens.categories.length],
    plotted: line.runs.map((run): Run => ({ firstHour: run[0], lastHour: run[1], value: percent ? Math.round(run[2] * 10000 / scale) / 100 : run[2] })) })),
  [row, tokens.categories, percent, scale]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      for (const line of lines) drawLine(ctx, frame, chartTokens, line.plotted, line.color, { until: state.playhead ?? undefined, held: true });
    },
    extent: x => {
      let range: [number, number] | null = null;
      for (const line of lines) {
        const shown = extentOf(line.plotted, x[0], x[1]);
        if (shown) range = range ? [Math.min(range[0], shown[0]), Math.max(range[1], shown[1])] : shown;
      }
      return range;
    },
  }], [lines]);
  const overlay = useCallback((ctx: CanvasRenderingContext2D, frame: PlotFrame, chartTokens: ChartTokens) => {
    const at = Math.min(frame.x[1], extent[1]);
    placeEndLabels(ctx, frame, chartTokens, lines.flatMap(line => {
      const end = lineEndHour(line.plotted, frame.x[0], at);
      const run = end === null ? null : plottedAt(line.plotted, end);
      return run ? [{ id: line.label, y: frame.toY(run.value), text: percent ? `${run.value}%` : String(run.value), color: line.color }] : [];
    }));
  }, [lines, percent, extent[1]]);
  const facts = hour === null ? [] : row.facts(hour);
  const shownHour = hour ?? (liveHead ? Math.floor(liveHead.read()) : extent[1]);
  return <div className="convoy-health-drill">
    {liveFacts ? <dl className="convoy-health-facts">{row.labels.map(label => <div key={label}><dt>{label}</dt><dd><LiveText live={liveFacts} at={label} /></dd></div>)}</dl>
      : facts.length ? <dl className="convoy-health-facts">{facts.map(fact => <div key={fact.label} data-detail={fact.detail}><dt>{fact.label}</dt><dd>{fact.value}</dd></div>)}</dl>
        : <p className="drill-empty">Nothing was recorded for this row at this date.</p>}
    <Legend label="Lines" items={lines.map(line => ({ id: line.label, label: line.label, color: line.color }))} />
    <div className="convoy-health-drill-chart">
      <TimeChart label={`${row.label} over time`} extent={extent} playhead={shownHour} liveHead={liveHead} playback="cursor" afterPlayhead hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
        branchStart={track.branch.firstHour} axes={{ xFrom: null, xTo: null, yMin: percent ? 0 : null, yMax: percent ? 100 : null, log: false }}
        valueTicks={percent ? [0, 25, 50, 75, 100] : undefined} formatValue={percent ? value => `${value}%` : undefined} padding={{ right: 64 }}
        layers={layers} overlay={overlay} active={active} pickOwner={row.id} revision={`${row.id}:${lines.length}`}
        tooltip={at => fitTooltip([{ detail: row.nameDetail, title: dayLabel(Math.floor(at)), rows: row.facts(Math.floor(at)) }], viewport)}
        onPick={at => onPick(Math.floor(at))} />
    </div>
  </div>;
}
