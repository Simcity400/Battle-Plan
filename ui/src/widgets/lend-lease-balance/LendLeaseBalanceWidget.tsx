import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { countryTagAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { useCallback, useMemo,  useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { ButterflyChart } from '../../charts/butterfly';
import type { ButterflyItem } from '../../charts/butterfly';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel, drawEndLabels, drawLine } from '../../charts/draw';
import Legend from '../../charts/Legend';
import { liveAt, mapLive } from '../../charts/live';
import { extentOfAll } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { useProduct } from '../../dashboard/products';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS, PARTNERS } from './definition';
import { LEND_LEASE_BALANCE, LEND_LEASE_PARTNERS, amountLabel, amountText, balanceRows, balanceSelection, barValue, emptyBalance, firstRecorded, partnersAt,
  rankedRows, totalsOverTime, validateLendLeaseBalance, validateLendLeasePartners, valueAt } from './lendLeaseBalanceData';
import type { BalanceMeasure, BalanceRow } from './lendLeaseBalanceData';
import './lendLeaseBalance.css';
import type { LendLeasePartnersReply } from '../../generated/dashboard';
import { drillAction } from '../../charts/drill/drillState.ts';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: readonly string[] = [];
type BalanceItem = ButterflyItem & { row: BalanceRow };

function useItems(runtime: WidgetProps['runtime'], track: WidgetProps['runtime']['tracks'][number] | undefined, rows: readonly BalanceRow[], hour: number, active: boolean) {
  const tokens = useChartTokens();
  const tags = useMemo(() => rows.map(row => row.tag), [rows]);
  const looks = useCountryLooks(runtime.client, track, tags, hour, active);
  const colour = useCountryColours(tags, looks.color, tokens, looks);
  return useMemo<BalanceItem[]>(() => rows.map(row => ({
    id: String(row.slot), label: looks.name(row.tag, hour), flag: looks.flag(row.tag, hour),
    countryFlag: { nameFailure: looks.nameFailure?.(row.tag, hour), flagDetail: looks.flagDetail?.(row.tag, hour), flag: looks.flag(row.tag, hour),
      flagReason: looks.flagReason?.(row.tag, hour), flagPending: looks.flagPending?.(row.tag, hour) },
    color: colour(row.tag, hour), left: barValue(row.received), right: barValue(row.sent),
    leftText: amountLabel(row.received), rightText: amountLabel(row.sent), ratioText: '', segment: String(row.slot), row,
  })), [rows, looks, colour, hour]);
}

const tooltipOf = (item: BalanceItem, hour: number | null): TooltipContent => ({ ...countryNameDetails(item.countryFlag), title: item.label,
  subtitle: hour === null ? undefined : dayLabel(hour), rows: [{ label: 'Sent', value: amountText(item.row.sent) }, { label: 'Received', value: amountText(item.row.received) }] });

export default function LendLeaseBalanceWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const measure: BalanceMeasure = options.measure === 'fuel' ? 'fuel' : 'industry';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const anchor = useRef<HTMLDivElement>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [picked, setPicked] = useState<{ branch: string; tag: string; name: string } | null>(null);
  const needsGroups = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const head = useHeadProduct(runtime, useGate(runtime, widget), LEND_LEASE_BALANCE, track ? {} : null, { active, validate: validateLendLeaseBalance });
  const data = head.data;
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  const hour = head.hour ?? Math.floor(runtime.clock.getHour());
  const selected = useMemo(() => selectedCountryKeys({ kind: 'selected' }, track), [track]);
  const bySlot = widget.countries.kind === 'selected' && selected.some(isSlotKey);
  const slotTags = useSlotTags(runtime.client, track, active && bySlot);
  const fallback = useMemo(() => bySlot ? selected.flatMap(key => countryTagAt(slotTags, countryFromKey(key), hour) ?? []) : selected, [bySlot, selected, slotTags, hour]);
  const all = useMemo(() => data ? balanceRows(data, measure) : [], [data, measure]);
  const chosen = useMemo(() => bySlot && !slotTags ? null : balanceSelection(widget.countries, fallback, groups.data, hour, track?.branch.lastHour ?? hour),
    [bySlot, slotTags, widget.countries, fallback, groups.data, hour, track?.branch.lastHour]);
  const rows = useMemo(() => chosen ? rankedRows(all, chosen) : [], [all, chosen]);
  const items = useItems(runtime, track, rows, hour, active);
  const slots = widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? widget.countries.group.count : items.length;
  const open = (item: BalanceItem) => {
    setPicked({ branch: track?.branch.id ?? '', tag: item.row.tag, name: item.label });
    dispatch({ type: 'open', breakdown: PARTNERS.id, hour, follow: drillControl.followsAt(hour) });
  };
  const shownPick = picked && picked.branch === track?.branch.id ? picked : null;
  const drillRegion = useRegion(widget, drillAction(shownPick ? drill : CLOSED));
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null && !!shownPick);
  const at = drillAt ?? hour;

  let main;
  if (!track) main = <WidgetState kind="empty" icon="chart-bars" message="Load a campaign to see lend-lease." />;
  else if (head.error) main = <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  else if (needsGroups && groups.error && !groups.data) main = <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  else if (head.read === 'off' && !data) main = <WidgetState kind="outside" />;
  else if (!data || !chosen) main = <WidgetState kind="loading" label="Loading lend-lease…" />;
  else if (data.missing) main = <WidgetState kind="unavailable" message={data.missing} />;
  else main = <div className="lend-lease-balance">
    <div className="lend-lease-caption">
      <span>{head.hour === null ? 'At the playhead' : dayLabel(head.hour)}</span>
    </div>
    {!items.length ? (() => {
      const empty = emptyBalance(all, chosen);
      return empty === 'zero' ? <WidgetState kind="empty" icon="chart-bars" message="No lend-lease yet." />
        : empty === 'members-unrecorded' ? <WidgetState kind="unavailable" message="This group's members were not recorded at this date." />
          : empty === 'partial' ? <WidgetState kind="unavailable" message="No lend-lease in the recorded histories; some of these countries have none at this date." />
            : <WidgetState kind="unavailable" message="No lend-lease history is recorded for these countries at this date." />;
    })()
      : <ButterflyChart key={`${track.source.id}:${track.branch.id}`} label="Lend-lease received and sent" items={items} slots={slots}
        leftLabel="◀ Received" rightLabel="Sent ▶" ratioLabel="" active={active} hoursPerSecond={hoursPerSecond}
        showValues={options.values !== false} showRatio={false}
        tooltip={item => fitTooltip([tooltipOf(item, head.hour)], viewport)} onSelect={item => open(item)}
        contextItems={item => [{ label: 'Show partners', run: () => open(item) }]} />}
  </div>;
  if (!track) return main;
  return <div ref={anchor} className="lend-lease-balance-host">
    <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null && !!shownPick} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={shownPick ? `${shownPick.name}: ${PARTNERS.label.toLowerCase()}` : PARTNERS.label} day={drillAt === null ? '' : dayLabel(drillAt)} canFollow onClose={() => dispatch({ type: 'close' })}
      main={<div ref={head.root} className="lend-lease-balance-host">{main}</div>}>
      {shownPick && <Partners widget={widget} runtime={runtime} active={active} tag={shownPick.tag} name={shownPick.name} hour={at} measure={measure}
        following={drill.follow && widget.time.mode === 'follow'} onPick={pick => dispatch({ type: 'pick', hour: pick })} />}
    </DrillHost>
  </div>;
}

function Partners({ widget, runtime, active, tag, name, hour, measure, following, onPick }: Pick<WidgetProps, 'widget' | 'runtime' | 'active'> & {
  tag: string; name: string; hour: number; measure: BalanceMeasure; following: boolean; onPick: (hour: number) => void;
}) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0])!;
  const asked = useProduct<LendLeasePartnersReply>(runtime.client, productRequest({ sourceId: track.source.id, revision: track.source.revision, product: LEND_LEASE_PARTNERS,
    params: { branchId: track.branch.id, holder: countryFromKey(tag), measure } }), active, validateLendLeasePartners, track.source);
  const data = asked.data;
  const rows = useMemo(() => data ? partnersAt(data, hour) : [], [data, measure, hour]);
  const items = useItems(runtime, track, rows, hour, active);
  const axisOffset = track.axisOffset ?? 0;
  const headAt = useCallback((position: number) => Math.min(track.branch.lastHour, Math.max(track.branch.firstHour, Math.floor(position + axisOffset))),
    [track.branch.firstHour, track.branch.lastHour, axisOffset]);
  const liveItems = useMemo(() => following && data ? mapLive(liveAt(runtime.clock, headAt, at => partnersAt(data, at)),
    shown => items.map(item => {
      const now = shown.find(row => row.slot === item.row.slot);
      return now ? { ...item, row: now, left: barValue(now.received), right: barValue(now.sent), leftText: amountLabel(now.received), rightText: amountLabel(now.sent) }
        : { ...item, left: null, right: null, leftText: '—', rightText: '—' };
    })) : undefined, [following, data, runtime.clock, headAt, measure, items]);
  const lines = useMemo(() => {
    if (!data) return [];
    const totals = totalsOverTime(data);
    return [{ label: 'Sent', runs: totals.sent, color: tokens.categories[0] }, { label: 'Received', runs: totals.received, color: tokens.categories[1] }];
  }, [data, measure, tokens.categories]);
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const layers = useMemo<ChartLayer[]>(() => [{ draw: (ctx, frame, chartTokens, state) => {
    const ends = lines.flatMap(line => { const end = drawLine(ctx, frame, chartTokens, line.runs, line.color, { until: state.playhead ?? undefined, held: true });
      return end ? [{ text: line.label, y: end.y, from: end.y, color: line.color }] : []; });
    drawEndLabels(ctx, frame, chartTokens, ends);
  }, extent: x => extentOfAll(lines.map(line => line.runs), x[0], x[1]) }], [lines]);
  if (asked.error) return <WidgetState kind="error" failure={asked.error} onRetry={asked.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading lend-lease…" />;
  const first = firstRecorded(data);
  const amounts = (at: number) => {
    const known = data.known.some(span => span.firstHour <= at && at <= span.lastHour);
    if (!known) return [{ label: 'Lend-lease histories were not recorded at this date', tone: 'muted' as const }];
    if (first === null || at < first) return [{ label: first === null ? 'No partner recorded' : `Not recorded before ${dayLabel(first)}`, tone: 'muted' as const }];
    const sent = valueAt(data.totals[0], at), received = valueAt(data.totals[1], at);
    return [{ label: 'Sent', value: sent === null ? '—' : amountText(sent) }, { label: 'Received', value: received === null ? '—' : amountText(received) }];
  };
  return <div className="lend-lease-partners">
    {!items.length ? <p className="drill-empty">No partner was recorded for {name} at this date.</p>
      : <div className="lend-lease-partner-bars"><ButterflyChart label={`Lend-lease partners of ${name}`} items={items} liveItems={liveItems}
        leftLabel="◀ Received" rightLabel="Sent ▶" ratioLabel="" empty={`No partner was recorded for ${name} at this date.`} active={active} hoursPerSecond={hoursPerSecond} showRatio={false}
        tooltip={item => fitTooltip([tooltipOf(item, hour)], viewport)} /></div>}
    {lines.some(line => line.runs.length) && <>
      <Legend label="Totals" items={lines.map(line => ({ id: line.label, label: line.label, color: line.color }))} />
      <div className="lend-lease-partner-time">
        <TimeChart label={`Lend-lease of ${name} over time`} extent={[track.branch.firstHour, track.branch.lastHour]} playhead={following ? null : hour}
          liveHead={liveHead} playback="cursor" afterPlayhead hoursPerSecond={hoursPerSecond} branchStart={track.branch.firstHour} padding={{ right: 72 }}
          axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} layers={layers} active={active} pickOwner={tag}
          revision={`${tag}:${measure}:${lines.map(line => line.runs.length).join(',')}`}
          tooltip={at => fitTooltip([{ title: dayLabel(Math.floor(at)), rows: amounts(Math.floor(at)) }], viewport)}
          onPick={at => onPick(Math.floor(at))} />
      </div>
    </>}
  </div>;
}
