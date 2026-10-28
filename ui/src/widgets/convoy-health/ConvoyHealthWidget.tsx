import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import HeatChart from '../../charts/heat/HeatChart';
import type { HeatPeriod, HeatRow } from '../../charts/heat/layout';
import { hatchStyle } from '../../charts/hatch';
import { jumpItem } from '../../charts/jump';
import { dayLabel, drawLine, placeEndLabels } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import type { PlotFrame } from '../../charts/frame';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { extentOf, lineEndHour, runAt } from '../../charts/series';
import type { Run } from '../../charts/series';
import type { ChartTokens } from '../../charts/tokens';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import Legend from '../../charts/Legend';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useChartTokens } from '../../charts/useChartTokens';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackSpeed } from '../../dashboard/playback';
import { preparedPending } from '../../dashboard/preparedPrefix';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { ConvoyHealthReply } from '../../generated/dashboard';
import { CONVOY_HEALTH, convoyPending, convoyRows, dayPeriods, heatRows, labelDates, periodPending, shownPeriods, tableKnown, validateConvoys } from './convoyData';
import { preparedAt } from '../../dashboard/economyHistory';
import type { ConvoyRow, ConvoyView } from './convoyData';
import { DETAILS, OPTIONS } from './definition';
import '../air-superiority-timeline/airSuperiorityTimeline.css';
import './convoyHealth.css';

const NO_WARNINGS: readonly string[] = [];
const VALIDATORS = { regions: validateConvoys('regions'), trade: validateConvoys('trade') };

/** Are convoys getting through: one country's delivery efficiency by sea region day by
 * day, or the trade efficiency each of its import deals gets, in Air superiority
 * timeline's heat strips. */
export default function ConvoyHealthWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const menu = useContextMenu('Convoy health');
  const options = resolveOptions(OPTIONS, widget.options);
  const view: ConvoyView = options.view === 'trade' ? 'trade' : 'regions';
  const count = typeof options.rowCount === 'number' && options.rowCount > 0 ? options.rowCount : 10;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? Math.floor(window.lastHour) : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: CONVOY_HEALTH,
    params: { branchId: track.branch.id, view, tag } } : null;
  const { data, error, retry } = useProduct<ConvoyHealthReply>(runtime.client, request, active, VALIDATORS[view], track?.source);
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  // Names only at the dates the rows look them up.
  const wanted = useMemo(() => data ? labelDates(data) : [], [data]);
  const names = useCountryNamesAt(runtime.client, track, wanted, active);
  // A name not (yet) read stays unavailable: country codes show only on hover.
  const countryName = useCallback((country: string | null, at: number) => country === null ? 'Country name unavailable'
    : names.get(nameKey(at, country)) ?? 'Country name unavailable', [names]);
  const days = useMemo(() => track ? dayPeriods(extent, track.branch.gaps) : [], [track, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  // Reads stop at the playhead and at what preparation has accepted: the day under way
  // reads the hour reached, a partly prepared day its prepared part.
  const pending = useMemo(() => data ? convoyPending(data, preparedPending(data), extent) : [], [data, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const periods = useMemo(() => track ? shownPeriods(days, track.branch.gaps, playhead, pending) : days, [track, days, playhead, pending]);
  const rows = useMemo(() => data && data.view === view ? convoyRows(data, { country: countryName }) : [], [data, view, countryName]);
  const color = tokens.categories[0];
  const heat = useMemo(() => heatRows(rows, periods, color, count, playhead), [rows, periods, color, count, playhead]);
  const labels = useMemo(() => rows.map(row => row.label), [rows]);
  const byId = useMemo(() => new Map(rows.map(row => [row.id, row])), [rows]);
  const tooltip = useCallback((row: HeatRow, period: HeatPeriod | null, hour: number) => {
    const entry = byId.get(row.id);
    const read = period?.readHour ?? null;
    const details = entry && read !== null ? entry.detail(read) : [];
    const missing = read === null && period && periodPending(period, pending) ? 'Still being prepared' : 'Not recorded';
    return { title: row.label, subtitle: dayLabel(period?.firstHour ?? hour), rows: [
      ...details.length ? details : [{ label: read === null || entry?.unknownAt(read) ? missing : entry?.kind === 'region' ? 'No bad convoy route here' : 'No deal this day', tone: 'muted' as const }],
      ...read !== null && details.length ? [{ label: `As of ${dayLabel(read)}`, tone: 'muted' as const }] : [],
    ] };
  }, [byId, pending]);
  const onMenu = useCallback((hour: number, x: number, y: number) => { if (track) menu.open(x, y, [jumpItem(runtime, track.id, hour)]); }, [track, menu, runtime]);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [picked, setPicked] = useState<string | null>(null);
  const onPick = useCallback((row: HeatRow, period: HeatPeriod | null, hour: number) => {
    // A day still being prepared has nothing to open yet.
    if (period && period.readHour === null && periodPending(period, pending)) return;
    setPicked(row.id);
    dispatch({ type: 'open', breakdown: DETAILS.id, hour: period?.readHour ?? hour, playing: runtime.clock.getSnapshot().playing, playhead });
  }, [runtime, playhead, pending]);
  const pickOwner = useMemo(() => ({ data, view, tag }), [data, view, tag]);
  const drilled = picked ? byId.get(picked) ?? null : null;
  const drillAt = drillHour(drill, playhead) ?? playhead ?? extent[1];

  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see convoys." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data || data.view !== view) return <WidgetState kind="loading" label="Loading convoys…" />;
  const shownHour = playhead ?? extent[1];
  if (!heat.length) {
    // Emptiness needs the prepared hours and the tables actually read.
    if (!preparedAt(data, shownHour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
    const tables = view === 'regions' ? [0] : [1, 2];
    return tables.every(index => tableKnown(data, index, shownHour))
      ? <WidgetState kind="empty" icon="anchor" message="No convoy activity at this date." />
      : <WidgetState kind="unavailable" message="Convoys were not recorded at this date." />;
  }
  const legend = [{ id: 'full', label: view === 'regions' ? 'Convoy efficiency (0–100%)' : 'Trade efficiency (0–100%)', color }];
  const main = <div className="air-timeline convoy-health">
    <div className="air-pattern-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: tokens.surface }} />{view === 'regions' ? 'No bad route' : 'No deal'}</span>
      <span><i aria-hidden="true" style={{ background: `color-mix(in srgb, ${tokens.error} ${tokens.gapOpacity * 100}%, ${tokens.surface})` }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(tokens)} />{view === 'trade' ? 'Efficiency not recorded' : 'Unknown'}</span>
    </div>
    <div className="air-timeline-chart"><HeatChart label={view === 'regions' ? 'Convoy efficiency by sea region' : 'Trade efficiency of imports'}
      rows={heat} periods={periods} labelCatalogue={labels} extent={extent} playhead={playhead} branchStart={track.branch.firstHour}
      legend={legend} afterPlayhead={widget.time.mode !== 'follow'} look="margin" active={active} hoursPerSecond={hoursPerSecond}
      revision={`${data.branchId}:${view}:${tag}:${heat.length}:${playhead}`} pending={pending}
      tooltip={tooltip} onContextMenu={onMenu} onPick={onPick} pickOwner={pickOwner} /></div>
    {menu.menu}
  </div>;
  return <div ref={anchor} className="convoy-health-host">
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!drilled} anchor={anchor}
      title={drilled?.label ?? DETAILS.label} day={dayLabel(drillAt)} following={drill.follow} canFollow={widget.time.mode === 'follow'}
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}>
      {drilled && <ConvoyDrill row={drilled} runtime={runtime} track={track} hour={drillAt} extent={extent} active={active} scale={data.scale}
        onPick={hour => dispatch({ type: 'pick', hour })} />}
    </DrillHost>
  </div>;
}

/** One row's own history (efficiency and its factors, or a shipment's convoys) and
 * its facts at the drill's date. */
function ConvoyDrill({ row, runtime, track, hour, extent, active, scale, onPick }: { row: ConvoyRow; runtime: WidgetRuntime; track: BoundTrack;
  hour: number; extent: [number, number]; active: boolean; scale: number; onPick: (hour: number) => void }) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const percent = row.unit === 'percent';
  const lines = useMemo(() => row.lines.map((line, index) => ({ ...line, color: tokens.categories[index % tokens.categories.length],
    plotted: line.runs.map((run): Run => ({ firstHour: run[0], lastHour: run[1], value: percent ? Number(BigInt(run[2]) * 10000n / BigInt(scale)) / 100 : Number(run[2]) })) })),
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
  // End labels sit in the right gutter, outside the plot's clip, so they are drawn after it.
  const overlay = useCallback((ctx: CanvasRenderingContext2D, frame: PlotFrame, chartTokens: ChartTokens) => {
    const at = Math.min(frame.x[1], extent[1]);
    placeEndLabels(ctx, frame, chartTokens, lines.flatMap(line => {
      const end = lineEndHour(line.plotted, frame.x[0], at);
      const run = end === null ? null : runAt(line.plotted, end);
      return run ? [{ id: line.label, y: frame.toY(run.value), text: percent ? `${run.value}%` : String(run.value), color: line.color }] : [];
    }));
  }, [lines, percent, extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const facts = row.detail(hour);
  return <div className="convoy-health-drill">
    {facts.length ? <dl className="convoy-health-facts">{facts.map(fact => <div key={fact.label}><dt>{fact.label}</dt><dd>{fact.value}</dd></div>)}</dl>
      : <p className="drill-empty">Nothing was recorded for this row at this date.</p>}
    <Legend label="Lines" items={lines.map(line => ({ id: line.label, label: line.label, color: line.color }))} />
    <div className="convoy-health-drill-chart">
      <TimeChart label={`${row.label} over time`} extent={extent} playhead={hour} playback="cursor" afterPlayhead hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
        branchStart={track.branch.firstHour} axes={{ xFrom: null, xTo: null, yMin: percent ? 0 : null, yMax: percent ? 100 : null, log: false }}
        valueTicks={percent ? [0, 25, 50, 75, 100] : undefined} formatValue={percent ? value => `${value}%` : undefined} padding={{ right: 64 }}
        layers={layers} overlay={overlay} active={active} pickOwner={row.id} revision={`${row.id}:${lines.length}`}
        tooltip={at => fitTooltip([{ title: dayLabel(Math.floor(at)), rows: row.detail(Math.floor(at)) }], viewport)}
        onPick={at => onPick(Math.floor(at))} />
    </div>
  </div>;
}

