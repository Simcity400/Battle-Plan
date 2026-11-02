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
import { extentOfAll } from '../../charts/series';
import type { Run } from '../../charts/series';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import Legend from '../../charts/Legend';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt.ts';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupReasonAt, preparedAt, tagAt } from '../../dashboard/economyHistory.ts';
import { MEMBERSHIP_PREPARING, membersKnown } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { preparedPending } from '../../dashboard/preparedPrefix';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { LendLeaseBalanceReply } from '../../generated/dashboard';
import { MEASURES, OPTIONS, PARTNERS } from './definition';
import { amountLabel, amountText, balanceAt, barValue, emptyBalance, firstRecorded, LEND_LEASE_BALANCE, rankedAt, validateLendLeaseBalance } from './lendLeaseBalanceData';
import type { BalanceMeasure, BalanceRow } from './lendLeaseBalanceData';
import '../equipment-stockpile/equipmentStockpile.css';
import './lendLeaseBalance.css';

const NO_WARNINGS: readonly string[] = [];
type BalanceItem = ButterflyItem & { row: BalanceRow };

/** Who props up whom: each country's cumulative lend-lease sent (right) and received
 * (left) in the game's own measures, ranked by both; a country opens its partners. */
export default function LendLeaseBalanceWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const playbackHour2 = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const measure: BalanceMeasure = options.measure === 'fuel' ? 'fuel' : 'industry';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour2);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const anchor = useRef<HTMLDivElement>(null);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]);
  const owner = useMemo(() => ({ client: runtime.client, scope }), [runtime.client, scope]);
  const [storedDrill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [storedPick, setPicked] = useState<{ owner: object; holder: string; tag: string } | null>(null);
  // A holder belongs to the recording it was picked in.
  const picked = storedPick?.owner === owner ? storedPick : null;
  const drill = picked ? storedDrill : CLOSED;
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
    countryFlag: { nameFailure: looks.nameFailure?.(row.tag, hour), flagDetail: looks.flagDetail?.(row.tag, hour), flag: looks.flag(row.tag, hour), flagReason: looks.flagReason?.(row.tag, hour), flagPending: looks.flagPending?.(row.tag, hour) },
    color: colour(row.tag, hour), left: barValue(row.received), right: barValue(row.sent),
    leftText: amountLabel(row.received, scale), rightText: amountLabel(row.sent, scale), ratioText: '', segment: row.holder, row,
  })), [rows, looks, colour, hour, scale]);
  const histories = useMemo(() => new Map(data?.histories.map(history => [history.holder, history]) ?? []), [data]);
  const slots = widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? widget.countries.group.count : items.length;
  const tooltip = (item: BalanceItem): TooltipContent => {
    const history = histories.get(item.row.holder);
    const first = history ? firstRecorded(history) : null;
    return { ...countryNameDetails(item.countryFlag), title: item.label, subtitle: dayLabel(hour), rows: [
      { label: 'Sent', value: amountText(item.row.sent, scale) }, { label: 'Received', value: amountText(item.row.received, scale) },
      ...first !== null ? [{ label: `Not recorded before ${dayLabel(first)}`, tone: 'muted' as const }] : [],
    ] };
  };
  const open = (item: BalanceItem) => {
    setPicked({ owner, holder: item.row.holder, tag: item.row.tag });
    dispatch({ type: 'open', breakdown: PARTNERS.id, hour, playing: runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  };
  const playbackHour = Math.floor(playbackHour2) + (track?.axisOffset ?? 0);
  const at = drillHour(drill, playbackHour) ?? hour;
  const pickedHistory = picked ? histories.get(picked.holder) : undefined;
  const pickedTag = pickedHistory ? tagAt(pickedHistory.tags, at) : null;
  const pickedNames = useCountryLabelsAt(runtime.client, track, pickedTag ? [{ tag: pickedTag, hour: at }] : [], active);
  const pickedName = pickedTag ? pickedNames.labelAt(at)(pickedTag).name : 'Country name unavailable';

  if (!track) return <WidgetState kind="empty" icon="chart-bars" message="Load a campaign to see lend-lease." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!data || !rows) return <>{groupFailure}<WidgetState kind="loading" label="Loading lend-lease…" /></>;
  if (!preparedAt(data, hour)) return <>{groupFailure}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  // A group whose membership at this hour is unrecorded or empty says so, never "none".
  const groupReason = groupReasonAt(widget.countries, groups.data, hour, track.branch.lastHour);
  if (groupReason) return <>{groupFailure}<WidgetState kind={groupReason.kind} message={groupReason.message} /></>;
  const main = <div className="lend-lease-balance">
    <div className="stockpile-caption-line">
      <span>{dayLabel(hour)}</span>
      <Segmented value={measure} options={MEASURES} label="Measure" onChange={value => setOption?.('measure', value)} />
    </div>
    {!items.length ? (() => {
      // A group's members at an hour still being prepared are not known yet: no row is not proof of none.
      if (!membersKnown(widget.countries, groups.data, track.branch.lastHour)(hour)) return <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
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
  return <div ref={anchor} className="lend-lease-balance-host">{groupFailure}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!picked} anchor={anchor}
      title={picked ? `${pickedName}: ${PARTNERS.label.toLowerCase()}` : PARTNERS.label} day={dayLabel(at)}
      following={drill.follow} canFollow={widget.time.mode === 'follow'} onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })}
      main={main}>
      {picked && <Partners widget={widget} runtime={runtime} active={active} holder={picked.holder} name={pickedName} measure={measure} hour={at}
        onPick={pick => dispatch({ type: 'pick', hour: pick })} />}
    </DrillHost>
  </div>;
}

/** One country's records of each partner at the drill's hour, and its totals over time. */
function Partners({ widget, runtime, active, holder, name, measure, hour, onPick }: {
  widget: WidgetProps['widget']; runtime: WidgetProps['runtime']; active: boolean; holder: string; name: string;
  measure: BalanceMeasure; hour: number; onPick: (hour: number) => void;
}) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
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
    .map(row => ({ id: row.holder, nameFailure: looks.nameFailure?.(row.tag, hour), label: looks.name(row.tag, hour), flag: looks.flag(row.tag, hour), color: colour(row.tag, hour),
      left: barValue(row.received), right: barValue(row.sent), leftText: amountLabel(row.received, scale), rightText: amountLabel(row.sent, scale),
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
  }, extent: x => extentOfAll(lines.map(line => line.plotted), x[0], x[1]) }], [lines, pending]);
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading lend-lease…" />;
  return <div className="lend-lease-partners">
    {!items.length ? <p className="drill-empty">No partner was recorded for {name} at this date.</p>
      : <div className="lend-lease-partner-bars"><ButterflyChart label={`Lend-lease partners of ${name}`} items={items} leftLabel="◀ Received" rightLabel="Sent ▶" ratioLabel=""
        active={active} hoursPerSecond={hoursPerSecond} showRatio={false}
        tooltip={item => { const row = rows.find(each => each.holder === item.id);
          return fitTooltip([{ ...countryNameDetails(item), title: item.label, subtitle: dayLabel(hour), rows: [{ label: 'Sent', value: amountText(row?.sent ?? null, scale) }, { label: 'Received', value: amountText(row?.received ?? null, scale) }] }], viewport); }} /></div>}
    {own && <>
      <Legend label="Totals" items={lines.map(line => ({ id: line.label, label: line.label, color: line.color }))} />
      <div className="lend-lease-partner-time">
        <TimeChart label={`Lend-lease of ${name} over time`} extent={[track.branch.firstHour, track.branch.lastHour]} playhead={hour} playback="cursor"
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
