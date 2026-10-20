import { useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import { ButterflyChart } from '../../charts/butterfly';
import type { ButterflyItem } from '../../charts/butterfly';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel, drawEndLabels, drawLine } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { pendingLayer } from '../../charts/prepared';
import { PreparedKey } from '../../charts/PreparedKey';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { Run } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import Legend from '../../charts/Legend';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupReasonAt, preparedAt } from '../../dashboard/economyHistory';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackSpeed } from '../../dashboard/playback';
import { preparedPending } from '../../dashboard/preparedPrefix';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { LendLeaseBalanceReply } from '../../generated/dashboard';
import { MEASURES, OPTIONS, PARTNERS } from './definition';
import { amountText, balanceAt, barValue, emptyBalance, firstRecorded, LEND_LEASE_BALANCE, rankedAt, validateLendLeaseBalance } from './lendLeaseBalanceData';
import type { BalanceMeasure, BalanceRow } from './lendLeaseBalanceData';
import '../equipment-stockpile/equipmentStockpile.css';
import './lendLeaseBalance.css';

const NO_WARNINGS: readonly string[] = [];
type BalanceItem = ButterflyItem & { row: BalanceRow };

/** Who props up whom: each country's cumulative lend-lease sent (right) and received
 * (left) in the game's own measures, ranked by both; a country opens its partners. */
export default function LendLeaseBalanceWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const measure: BalanceMeasure = options.measure === 'fuel' ? 'fuel' : 'industry';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [picked, setPicked] = useState<{ holder: string; tag: string } | null>(null);
  const needsGroups = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: LEND_LEASE_BALANCE,
    params: { branchId: track.branch.id } } : null;
  const { data, error, retry } = useProduct<LendLeaseBalanceReply>(runtime.client, request, active, validateLendLeaseBalance, track?.source);
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  const fallback = useMemo(() => selectedTags({ kind: 'selected' }, track), [track]);
  const rows = useMemo(() => data ? rankedAt(data, measure, hour, widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour) : null,
    [data, measure, hour, widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const tags = useMemo(() => rows?.map(row => row.tag) ?? [], [rows]);
  const looks = useCountryLooks(runtime.client, track, tags, hour, active);
  const colour = useCountryColours(tags, looks.color, tokens, looks);
  const scale = data?.scale ?? 100000;
  const items = useMemo<BalanceItem[]>(() => (rows ?? []).map(row => ({
    id: row.holder, label: looks.name(row.tag, hour), flag: looks.flag(row.tag, hour),
    countryFlag: { flag: looks.flag(row.tag, hour), flagReason: looks.flagReason?.(row.tag, hour), flagPending: looks.flagPending?.(row.tag, hour) },
    color: colour(row.tag, hour), left: barValue(row.received), right: barValue(row.sent),
    leftText: amountText(row.received, scale), rightText: amountText(row.sent, scale), ratioText: '', segment: row.holder, row,
  })), [rows, looks, colour, hour, scale]);
  const histories = useMemo(() => new Map(data?.histories.map(history => [history.holder, history]) ?? []), [data]);
  const slots = widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? widget.countries.group.count : items.length;
  const tooltip = (item: BalanceItem): TooltipContent => {
    const history = histories.get(item.row.holder);
    const first = history ? firstRecorded(history) : null;
    return { title: item.label, subtitle: dayLabel(hour), rows: [
      { label: 'Sent', value: item.rightText }, { label: 'Received', value: item.leftText },
      ...first !== null ? [{ label: `Not recorded before ${dayLabel(first)}`, tone: 'muted' as const }] : [],
    ] };
  };
  const open = (item: BalanceItem) => {
    setPicked({ holder: item.row.holder, tag: item.row.tag });
    dispatch({ type: 'open', breakdown: PARTNERS.id, hour, playing: runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  };
  const playbackHour = Math.floor(runtime.clock.getHour()) + (track?.axisOffset ?? 0);
  const at = drillHour(drill, playbackHour) ?? hour;

  if (!track) return <WidgetState kind="empty" icon="chart-bars" message="Load a campaign to see lend-lease." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (needsGroups && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (!data || !rows) return <WidgetState kind="loading" label="Loading lend-lease…" />;
  if (!preparedAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  // A group whose membership at this hour is unrecorded or empty says so, never "none".
  const groupReason = groupReasonAt(widget.countries, groups.data, hour, track.branch.lastHour);
  if (groupReason) return <WidgetState kind={groupReason.kind} message={groupReason.message} />;
  const main = <div className="lend-lease-balance">
    <div className="stockpile-caption-line">
      <span>{dayLabel(hour)}</span>
      <Segmented value={measure} options={MEASURES} label="Measure" onChange={value => setOption?.('measure', value)} />
    </div>
    {!items.length ? (() => {
      const empty = emptyBalance(data, measure, hour, widget.countries, fallback, groups.data, track.branch.lastHour);
      return empty === 'zero' ? <WidgetState kind="empty" icon="chart-bars" message="No lend-lease yet." />
        : empty === 'unrecorded' ? <WidgetState kind="unavailable" message="Lend-lease histories were not recorded at this date." />
          : empty === 'members-unrecorded' ? <WidgetState kind="unavailable" message="This group's members were not recorded at this date." />
          : empty === 'partial' ? <WidgetState kind="unavailable" message="No lend-lease in the recorded histories; some of these countries have none at this date." />
            : <WidgetState kind="unavailable" message="No lend-lease history is recorded for these countries at this date." />;
    })()
      : <ButterflyChart key={`${track.source.id}:${track.branch.id}`} label="Lend-lease received and sent" items={items} slots={slots}
        leftLabel="◀ Received" rightLabel="Sent ▶" ratioLabel="" active={active} hoursPerSecond={hoursPerSecond}
        showValues={options.values !== false} showRatio={false}
        tooltip={item => fitTooltip([tooltip(item)], viewport)} onSelect={item => open(item)}
        contextItems={item => { const history = histories.get(item.row.holder); const first = history && firstRecorded(history);
          return [{ label: 'Show partners', run: () => open(item) }, ...first != null ? [jumpItem(runtime, track.id, first)] : []]; }} />}
  </div>;
  return <div ref={anchor} className="lend-lease-balance-host">
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!picked} anchor={anchor}
      title={picked ? `${looks.name(picked.tag, at)}: ${PARTNERS.label.toLowerCase()}` : PARTNERS.label} day={dayLabel(at)}
      following={drill.follow} canFollow={widget.time.mode === 'follow'} onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })}
      main={main}>
      {picked && <Partners widget={widget} runtime={runtime} active={active} holder={picked.holder} tag={picked.tag} measure={measure} hour={at}
        onPick={pick => dispatch({ type: 'pick', hour: pick })} />}
    </DrillHost>
  </div>;
}

/** One country's records of each partner at the drill's hour, and its totals over time. */
function Partners({ widget, runtime, active, holder, tag, measure, hour, onPick }: {
  widget: WidgetProps['widget']; runtime: WidgetProps['runtime']; active: boolean; holder: string; tag: string;
  measure: BalanceMeasure; hour: number; onPick: (hour: number) => void;
}) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0])!;
  const request = { sourceId: track.source.id, revision: track.source.revision, product: LEND_LEASE_BALANCE, params: { branchId: track.branch.id, holder } };
  const { data, error, retry } = useProduct<LendLeaseBalanceReply>(runtime.client, request, active, validateLendLeaseBalance, track.source);
  const totals = useProduct<LendLeaseBalanceReply>(runtime.client, { ...request, params: { branchId: track.branch.id } }, active, validateLendLeaseBalance, track.source);
  const rows = useMemo(() => data?.histories.map(history => balanceAt(history, measure, hour)).filter((row): row is BalanceRow => row !== null) ?? [], [data, measure, hour]);
  const tags = useMemo(() => rows.map(row => row.tag), [rows]);
  const looks = useCountryLooks(runtime.client, track, tags, hour, active);
  const colour = useCountryColours(tags, looks.color, tokens, looks);
  const scale = data?.scale ?? 100000;
  const items = useMemo(() => rows.sort((a, b) => { const x = (a.sent ?? 0n) + (a.received ?? 0n), y = (b.sent ?? 0n) + (b.received ?? 0n); return x === y ? 0 : x > y ? -1 : 1; })
    .map(row => ({ id: row.holder, label: looks.name(row.tag, hour), flag: looks.flag(row.tag, hour), color: colour(row.tag, hour),
      left: barValue(row.received), right: barValue(row.sent), leftText: amountText(row.received, scale), rightText: amountText(row.sent, scale),
      ratioText: '', segment: row.holder })), [rows, looks, colour, hour, scale]);
  const own = totals.data?.histories.find(history => history.holder === holder);
  const offset = measure === 'industry' ? 0 : 1;
  const lines = useMemo(() => own ? [{ label: 'Sent', runs: own.values[offset], color: tokens.categories[0] }, { label: 'Received', runs: own.values[2 + offset], color: tokens.categories[1] }]
    .map(line => ({ ...line, plotted: line.runs.map((run): Run => ({ firstHour: run[0], lastHour: run[1], value: Number(BigInt(run[2])) / scale })) })) : [],
  [own, offset, tokens.categories, scale]);
  const pending = totals.data ? preparedPending(totals.data) : [];
  const layers = useMemo<ChartLayer[]>(() => [pendingLayer(pending), { draw: (ctx, frame, chartTokens, state) => {
    const ends = lines.flatMap(line => { const end = drawLine(ctx, frame, chartTokens, line.plotted, line.color, { until: state.playhead ?? undefined, held: true });
      return end ? [{ text: line.label, y: end.y, from: end.y, color: line.color }] : []; });
    drawEndLabels(ctx, frame, chartTokens, ends);
  } }], [lines, pending]);
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading lend-lease…" />;
  return <div className="lend-lease-partners">
    {!items.length ? <p className="drill-empty">No partner was recorded for {tag} at this date.</p>
      : <div className="lend-lease-partner-bars"><ButterflyChart label={`Lend-lease partners of ${tag}`} items={items} leftLabel="◀ Received" rightLabel="Sent ▶" ratioLabel=""
        active={active} hoursPerSecond={hoursPerSecond} showRatio={false}
        tooltip={item => fitTooltip([{ title: item.label, subtitle: dayLabel(hour), rows: [{ label: 'Sent', value: item.rightText }, { label: 'Received', value: item.leftText }] }], viewport)} /></div>}
    {own && <>
      <Legend label="Totals" items={lines.map(line => ({ id: line.label, label: line.label, color: line.color }))} />
      <div className="lend-lease-partner-time">
        <TimeChart label={`Lend-lease of ${tag} over time`} extent={[track.branch.firstHour, track.branch.lastHour]} playhead={hour} playback="cursor"
          afterPlayhead hoursPerSecond={hoursPerSecond} branchStart={track.branch.firstHour} padding={{ right: 72 }}
          axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} layers={layers} active={active} pickOwner={holder}
          revision={`${holder}:${measure}:${lines.length}:${pending.length}`}
          tooltip={at => { const point = Math.floor(at); const row = own && balanceAt(own, measure, point);
            return fitTooltip([{ title: dayLabel(point), rows: [{ label: 'Sent', value: amountText(row?.sent ?? null, scale) }, { label: 'Received', value: amountText(row?.received ?? null, scale) }] }], viewport); }}
          onPick={at => onPick(Math.floor(at))} />
      </div>
      <PreparedKey pending={pending} />
    </>}
  </div>;
}
