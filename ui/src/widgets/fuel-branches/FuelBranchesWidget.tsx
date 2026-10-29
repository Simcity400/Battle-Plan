import { useCallback, useId, useMemo, useReducer, useRef } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import FuelConsumptionDrill from './FuelConsumptionDrill';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel, drawEndLabels, drawLine } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { pendingLayer } from '../../charts/prepared';
import { PreparedKey } from '../../charts/PreparedKey';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip, tooltipText } from '../../charts/tooltip';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useRowTooltips } from '../../charts/useRowTooltips';
import Legend from '../../charts/Legend';
import CountryComparisons from '../../dashboard/CountryComparisons';
import type { ComparisonProps } from '../../dashboard/CountryComparisons';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { preparedAt } from '../../dashboard/economyHistory';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { preparedPending } from '../../dashboard/preparedPrefix';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { FuelBranchesReply } from '../../generated/dashboard';
import { fuelDecimals, formatFuelPart } from '../fuel/fuelData';
import { CONSUMPTION, OPTIONS } from './definition';
import { branchesAt, changeHours, chartCountryAt, consumersKnown, countryAt, emptyTimeView, FUEL_BRANCHES, memberAt, memberRatioRuns, percentText, ratioAxis, readAt, validateFuelBranches } from './fuelBranchesData';
import type { BranchAt } from './fuelBranchesData';
import '../resource-balance/resourceBalance.css';
import './fuelBranches.css';

const NO_WARNINGS: readonly string[] = [];

export default function FuelBranchesWidget(props: WidgetProps) {
  return <CountryComparisons {...props} component={FuelBranchesCountry} />;
}

/** One country's fuel consumers: bars of received against requested at the read
 * shown, or each branch's share received over time. */
function FuelBranchesCountry({ widget, runtime, active, onDiagnostics, within }: ComparisonProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const menu = useContextMenu('Fuel by branch actions');
  const owner = useId();
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: FUEL_BRANCHES,
    params: { branchId: track.branch.id, tags: [tag] } } : null;
  const { data, error, retry } = useProduct<FuelBranchesReply>(runtime.client, request, active, validateFuelBranches, track?.source);
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  const digits = fuelDecimals(options);
  const amount = useCallback((raw: bigint | null) => raw === null ? '—' : formatFuelPart(raw, BigInt(data?.scale ?? 100000), digits), [data?.scale, digits]);
  const member = !within || within.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const country = data && tag ? countryAt(data, tag, hour) : null;
  const rows = useMemo(() => data && country && member ? branchesAt(data, country, hour) : [], [data, country, member, hour]);
  const read = country ? readAt(country, hour) : null;
  const tooltip = useCallback((row: BranchAt): TooltipContent => {
    const lines: TooltipRow[] = [
      { label: 'Requested', value: amount(row.requested) },
      { label: 'Received', value: amount(row.received) },
    ];
    if (row.unmet !== null && row.unmet > 0n) lines.push({ label: 'Unmet', value: amount(row.unmet) });
    if (row.priority !== null) lines.push({ label: 'Priority', value: String(row.priority) });
    if (read !== null) lines.push({ label: 'As of', value: dayLabel(read), tone: 'muted' });
    if (row.requested === 0n) lines.push({ label: 'No fuel requested', tone: 'muted' });
    return { title: row.name, rows: lines };
  }, [amount, read]);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const open = () => dispatch({ type: 'open', breakdown: CONSUMPTION.id, hour, playing: runtime.clock.getSnapshot().playing,
    playhead: widget.time.mode === 'follow' ? hour : null });
  const rowTips = useRowTooltips<number>({ owner, active, revision: `${hour}:${digits}`,
    content: index => { const row = rows.find(item => item.index === index); return row ? fitTooltip([tooltip(row)], viewport) : null; },
    onMenu: (_, x, y) => { if (track && read !== null) menu.open(x, y, [jumpItem(runtime, track.id, read)]); } });

  if (!track) return <WidgetState kind="empty" icon="droplet" message="Load a campaign to see fuel." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading fuel by branch…" />;
  if (options.view === 'time') return <FuelBranchesOverTime data={data} tag={tag} hour={hour} widget={widget} runtime={runtime} active={active} amount={amount} within={within} />;
  if (!preparedAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!member) return <WidgetState kind="unavailable" message="This country is not in the group at this date." />;
  if (!country || !rows.length) return consumersKnown(data, hour)
    ? <WidgetState kind="empty" icon="droplet" message={`${tag} requested no fuel at this date.`} />
    : <WidgetState kind="unavailable" message="Fuel consumers aren’t known for every country at this date." />;
  // One scale for every branch: the largest amount shown.
  let largest = 0n;
  for (const row of rows) for (const value of [row.requested, row.received]) if (value !== null && value > largest) largest = value;
  const width = (raw: bigint) => largest > 0n ? `${Number(raw * 100000n / largest) / 1000}%` : '0%';
  const main = <div className="resource-balance fuel-branches">
    <ol className="resource-bars" aria-label={`Fuel by branch on ${dayLabel(hour)}`}>
      {rows.map(row => {
        const asked = row.requested ?? 0n, got = row.received ?? 0n;
        const delivered = got < asked ? got : asked;
        const content = tooltip(row);
        const props = rowTips.rowProps(row.index);
        return <li key={row.index} {...props} role="button" onClick={open} onKeyDown={event => {
          if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); open(); } else props.onKeyDown(event);
        }}>
          <span id={rowTips.descriptionId(row.index)} className="sr-only">{tooltipText(content)}</span>
          <span className="resource-name"><span>{row.name}</span></span>
          {row.requested === null || row.received === null ? <span className="resource-missing">Not recorded</span>
            : row.requested === 0n ? <><span className="resource-track" aria-hidden="true" /><b data-tooltip="No fuel requested">—</b></>
              : <>
                <span className="resource-track fuel-branch-track" aria-hidden="true">
                  <span className="resource-bar">
                    <i style={{ width: width(delivered), background: tokens.categories[0] }} />
                    {row.unmet !== null && row.unmet > 0n && <i style={{ width: width(row.unmet), background: tokens.error, marginLeft: 1 }} />}
                  </span>
                  {row.unmet !== null && row.unmet > 0n && <small className="fuel-branch-unmet">unmet<span> {amount(row.unmet)}</span></small>}
                </span>
                <b><span className="fuel-branch-received">{amount(row.received)}</span><span className="fuel-branch-requested"> / {amount(row.requested)}</span></b>
              </>}
        </li>;
      })}
    </ol>
    {menu.menu}
  </div>;
  const at = drillHour(drill, hour) ?? hour;
  const extent: [number, number] = [track.branch.firstHour, track.branch.lastHour];
  return <div ref={anchor} className="fuel-branches-host">
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null} anchor={anchor} title={CONSUMPTION.label}
      day={dayLabel(at)} following={drill.follow} canFollow={widget.time.mode === 'follow'} onClose={() => dispatch({ type: 'close' })}
      onFollow={() => dispatch({ type: 'follow' })} main={main}>
      <FuelConsumptionDrill runtime={runtime} track={track} tag={tag} hour={at} extent={extent} active={active} digits={digits}
        onPick={pick => dispatch({ type: 'pick', hour: pick })} />
    </DrillHost>
  </div>;
}

/** Each branch's received share of what it requested, as a line per branch on a
 * fixed 0–100% scale; hours without a request or a read are gaps. */
function FuelBranchesOverTime({ data, tag, hour, widget, runtime, active, amount, within }: {
  data: FuelBranchesReply; tag: string; hour: number; widget: WidgetProps['widget']; runtime: WidgetProps['runtime']; active: boolean;
  amount: (raw: bigint | null) => string; within: ComparisonProps['within'];
}) {
  const options = resolveOptions(OPTIONS, widget.options);
  const viewport = useViewportHeight();
  const menu = useContextMenu('Fuel by branch actions');
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0])!;
  const window = resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const extent: [number, number] = widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour];
  // A reused tag is a different country: draw the holder shown at the playback date.
  const country = chartCountryAt(data, tag, hour);
  const lines = useMemo(() => country ? [...country.branches].sort((a, b) => a.index - b.index).map((branch, index) => ({
    branch, runs: memberRatioRuns(branch, within), color: tokens.categories[index % tokens.categories.length], name: data.consumerNames[branch.index] ?? 'Branch name unavailable' })) : [],
  [country, tokens.categories, data.consumerNames, within]);
  // Consumers can receive more than they asked for: the axis shows every recorded ratio.
  const axis = useMemo(() => ratioAxis(lines), [lines]);
  const pending = preparedPending(data);
  const ends = options.endLabels !== false;
  const layers = useMemo<ChartLayer[]>(() => [pendingLayer(pending), {
    draw: (ctx, frame, chartTokens, state) => {
      const labels = lines.flatMap(line => {
        const end = drawLine(ctx, frame, chartTokens, line.runs, line.color, { until: state.playhead ?? undefined, held: true });
        const last = line.runs.filter(run => run.firstHour <= (state.playhead ?? Infinity)).at(-1);
        return ends && end && last ? [{ text: percentText(last.value), y: end.y, from: end.y, color: line.color }] : [];
      });
      drawEndLabels(ctx, frame, chartTokens, labels);
    },
  }], [lines, pending, ends]);
  const keyHours = useMemo(() => country ? changeHours(country).filter(at => memberAt(within, at)) : [], [country, within]);
  if (!country || !lines.length) {
    const empty = emptyTimeView(data, extent[0], hour);
    return empty === 'pending' ? <WidgetState kind="loading" label="This range is still being prepared." />
      : empty === 'none' ? <WidgetState kind="empty" icon="droplet" message={`${tag} requested no fuel so far.`} />
        : empty === 'partial' ? <WidgetState kind="unavailable" message={`${tag} requested no fuel when fuel consumers were known. Other hours aren’t known.`} />
          : <WidgetState kind="unavailable" message="Fuel consumers aren’t known for every country." />;
  }
  const tooltip = (at: number) => {
    if (!memberAt(within, at)) return fitTooltip([{ title: dayLabel(at), rows: [{ label: 'Not in the group at this date', tone: 'muted' as const }] }], viewport);
    const rows = branchesAt(data, country, at);
    return fitTooltip([{ title: dayLabel(at), rows: rows.map(row => ({
      label: row.name, strong: true,
      value: row.requested === null || row.received === null ? 'Not recorded' : row.requested === 0n ? 'No fuel requested'
        : `${percentText(Number(row.received * 1_000_000n / row.requested) / 10_000)} · ${amount(row.received)} / ${amount(row.requested)}` })) }], viewport);
  };
  return <div className="fuel-branches-time">
    <Legend label="Branches" items={lines.map(line => ({ id: String(line.branch.index), label: line.name, color: line.color }))} />
    <div className="fuel-branches-chart">
      <TimeChart label={`Fuel received by branch of ${tag}`} extent={extent} playhead={following && window ? window.lastHour : null}
        playback={options.playback === 'cursor' ? 'cursor' : options.playback === 'window' ? 'window' : 'draw-in'}
        afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: 0, yMax: axis.top, log: false }} valueTicks={axis.ticks}
        formatValue={value => `${value}%`} padding={ends ? { right: 64 } : undefined}
        layers={layers} active={active} keyHours={keyHours} pickOwner={country}
        revision={`${data.branchId}:${country.holder}:${lines.length}:${ends}:${pending.length}`}
        tooltip={at => tooltip(Math.floor(at))}
        onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, at)])} />
    </div>
    <PreparedKey pending={pending} />
    {menu.menu}
  </div>;
}
