import type { CountryAt, Gate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart } from '../../charts/drill/DrillParts';
import type { LiveDrillParts } from '../../charts/drill/DrillParts';
import { mapLive } from '../../charts/live';
import { runAt } from '../../charts/series';
import type { Span } from '../../charts/series';
import { useChartTokens } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetBase } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { BURN_PARTS, FUEL_DAYS, GAIN_PARTS, GAIN_SCALE, PART_SCALE, QUANTITY_SCALE, formatFuelPart, fuelPartShare, fuelParts, partsStructure,
  scaledSummary, summaryRuns, validateCountryFuel, validateFuelAwardHistory, validateFuelAwards } from './fuelData';
import { detailWindows, fuelRecipientFor, joinedRuns, readBasis, sameFuelActor, selectFuelRecipient } from './fuelDrill';
import type { FuelBasis, FuelRecipientSelection } from './fuelDrill';
import { useFuelAt, usePartSeries } from './fuelReads';
import type { CountryFuelReply, FuelAwardHistoryReply, FuelAwardPart, FuelAwardsReply } from '../../generated/dashboard';
import { useChangedState } from '../../dashboard/changedState.ts';
import { productRequest } from '../../dashboard/request.ts';

type HistoryChart = Omit<Parameters<typeof BreakdownChart>[0], 'series' | 'axes' | 'label'>;
export interface DrillChart extends HistoryChart { detailHours: number }

export function useDetailView(start: number, end: number, detailHours: number) {
  const [visible, setVisible] = useChangedState<readonly [number, number]>([start, end]);
  const onViewChange = useCallback((x: readonly [number, number]) => setVisible(before => {
    const block = (hour: number) => Math.floor(hour / detailHours);
    return block(before[0]) === block(x[0]) && block(before[1]) === block(x[1]) ? before : x;
  }), [detailHours]);
  return [visible, onViewChange] as const;
}

function partRows(reply: CountryFuelReply | undefined, measure: 'gain' | 'burn', digits: number, color: (id: string) => string | undefined) {
  const read = reply?.read;
  const values = reply && read ? fuelParts(reply, read, measure, digits) : null;
  if (!values) return null;
  const parts: DrillPart[] = values.parts.map(part => ({ id: part.id, label: part.label, color: color(part.id), raw: part.raw,
    value: Number(part.raw) / Number(values.scale), text: formatFuelPart(part.raw, values.scale, digits), shareText: fuelPartShare(part.raw, values.total),
    muted: part.muted, selectable: measure === 'burn' && part.consumer !== undefined && !!read?.day?.awards }));
  return { values, parts, total: Number(values.total) / Number(values.scale), totalText: formatFuelPart(values.total, values.scale, digits) };
}

export default function FuelBreakdown({ runtime, gate, widget, track, tag, measure, hour, follows, within, selection: retained, onSelection, digits, campaign, aliases, detailHours, history, ...chart }: {
  runtime: WidgetRuntime; gate: Gate; widget: WidgetBase; track: BoundTrack; tag: string; measure: 'gain' | 'burn'; hour: number | null; follows: boolean; digits: number;
  campaign: string; aliases?: ReadonlyMap<string, readonly Span[]>; history: CountryAt['history'];
  selection: FuelRecipientSelection | null; onSelection: (selection: FuelRecipientSelection | null) => void;
} & DrillChart) {
  const tokens = useChartTokens();
  const shown = useShownHour();
  const ids = useMemo(() => measure === 'gain' ? [...GAIN_PARTS, ['incoming', 'incoming'] as const] : [...BURN_PARTS], [measure]);
  const color = useCallback((id: string) => {
    const at = ids.findIndex(([, part]) => part === id);
    return at < 0 ? undefined : tokens.categories[at % tokens.categories.length];
  }, [ids, tokens.categories]);
  const at = useFuelAt(runtime, widget, tag, hour, follows, chart.active, useCallback(reply => partsStructure(reply, measure), [measure]));
  const reply = at.data;
  useEffect(() => { shown(at.shownAt()); });
  const live = useMemo<LiveDrillParts>(() => mapLive(at.live, value => {
    const rows = partRows(value, measure, digits, color);
    shown(at.shownAt());
    return rows ? { parts: rows.parts, total: rows.total, totalText: rows.totalText } : { parts: [] };
  }), [at, measure, digits, color, shown]);
  const rows = useMemo(() => partRows(reply, measure, digits, color), [reply, measure, digits, color]);
  const selected = fuelRecipientFor(retained, campaign, track.branch.id, tag, aliases);
  const [pickedConsumer, setConsumer] = useState<number | null>(null);
  useEffect(() => { setConsumer(null); }, [chart.pickOwner]);
  const consumer = selected?.consumer ?? pickedConsumer;
  const basis = selected?.basis ?? (reply?.read?.day?.awards ? readBasis(track.branch.id, tag, reply.read) : null);
  const series = usePartSeries(runtime, gate, track, FUEL_DAYS, ids.map(([name]) => name), history, chart.active);
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const hoverAt = gate.recorded(hoverHour);
  const hoverAlias = hoverAt === null ? null : gate.country?.aliasAt(hoverAt) ?? null;
  const hovered = useProduct<CountryFuelReply>(runtime.client, hoverAt !== null && hoverAlias !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'country-fuel', params: { branchId: track.branch.id, country: countryFromKey(hoverAlias), hour: hoverAt } }) : null, chart.active, validateCountryFuel, track.source);
  const amountAt = useCallback((id: string, transfer: boolean) => (when: number) => {
    const held = hovered.data && hoverHour === Math.floor(when) ? hovered.data : reply && hour === Math.floor(when) ? reply : null;
    if (!held?.read) return held ? held.missing ?? 'Not recorded' : 'Loading…';
    const values = fuelParts(held, held.read, measure, digits);
    const part = values?.parts.find(item => item.id === id) ?? (transfer ? values?.transfer : null);
    return part && values ? formatFuelPart(part.raw, transfer ? BigInt(held.quantityScale) : values.scale, digits) : 'Not recorded';
  }, [hovered.data, hoverHour, reply, hour, measure, digits]);
  const parts = useMemo(() => ids.map(([name, id], index) => {
    const transfer = id === 'incoming';
    const summary = scaledSummary(series.summaries[index], transfer ? QUANTITY_SCALE : measure === 'gain' ? GAIN_SCALE : PART_SCALE);
    const label = id === 'incoming' ? reply?.transferLabels[0].template?.replace(/[:：]?\s*\{NUM\}.*$/, '') || 'Incoming lend-lease'
      : rows?.values.parts.find(part => part.id === id)?.label ?? (id === 'unidentified' ? 'Other' : name);
    return { id, transfer, summary, series: { id, label, color: color(id) ?? tokens.muted, runs: summaryRuns(summary), amount: amountAt(id, transfer),
      missingAt: amountAt(id, transfer) } satisfies BreakdownSeries };
  }), [ids, series.summaries, reply, rows, color, tokens.muted, amountAt, measure]);
  const own = parts.filter(part => !part.transfer && part.summary), incoming = parts.filter(part => part.transfer && part.summary);
  const basisAt = gate.recorded(basis?.hour);
  const awards = useProduct<FuelAwardsReply>(runtime.client, basis && basisAt !== null && measure === 'burn' && consumer !== null ? productRequest({ sourceId: track.source.id,
    revision: track.source.revision, product: 'country-fuel-awards', params: { branchId: basis.branchId, country: countryFromKey(basis.tag), hour: basisAt } }) : null,
  chart.active, validateFuelAwards, track.source);
  const recipients = awards.data && basis && awards.data.readHour === basis.hour && awards.data.tag === basis.tag ? awards.data : null;
  const recipient = selected && recipients?.recipients.find(part => part.key === selected.key);
  const listed = useMemo(() => (recipients?.recipients ?? []).filter(part => part.kind === consumer), [recipients, consumer]);
  const consumerTotal = recipients && consumer !== null ? BigInt(recipients.receivedRaw[consumer]) : null;
  const assigned = useMemo(() => listed.reduce((sum, part) => sum + BigInt(part.amountRaw), 0n), [listed]);
  const gain = BigInt(reply?.gainScale ?? 100000);
  const recipientParts = useMemo(() => listed.map(part => {
    const raw = BigInt(part.amountRaw);
    return { id: part.key, label: part.recordedName ?? 'Name unavailable', raw, value: Number(raw) / Number(gain), text: formatFuelPart(raw, gain),
      shareText: consumerTotal === null ? undefined : fuelPartShare(raw, consumerTotal), muted: part.recordedName === null, detail: part.nameReason ?? undefined };
  }), [listed, gain, consumerTotal]);
  const unattributed = consumerTotal !== null && assigned !== consumerTotal ? { label: 'Other', raw: consumerTotal - assigned,
    value: Number(consumerTotal - assigned) / Number(gain), text: formatFuelPart(consumerTotal - assigned, gain), shareText: fuelPartShare(consumerTotal - assigned, consumerTotal) } : undefined;
  const consumerName = consumer === null ? 'Consumer' : reply?.consumerNames[consumer] ?? 'Consumer';
  const chartProps = { ...chart, within, onSummaryView: series.onSummaryView, onHover: setHoverHour, selectedHour: hour };

  return <div ref={at.root}>
    {at.error && <WidgetState kind="error" what="fuel accounting" failure={at.error} onRetry={at.retry} />}
    {series.error && <WidgetState kind="error" what="fuel history" failure={series.error} onRetry={series.retry} />}
    {retained && !selected && <p className="drill-empty">Choose a recipient again.</p>}
    {selected ? <>
      <button type="button" className="button small quiet" onClick={() => { setConsumer(selected.consumer); onSelection(null); }}>All consumer recipients</button>
      <strong>{recipient?.recordedName ?? selected.name ?? 'Name unavailable'}</strong>
      <p className="drill-empty">{reply?.consumerNames[selected.consumer] ?? 'Consumer'}</p>
      {!sameFuelActor(selected.basis, reply?.read) && <p className="drill-empty">Country unavailable at this time.</p>}
      <RecipientHistory runtime={runtime} gate={gate} track={track} basis={selected.basis} recipientKey={selected.key} name={recipient?.recordedName ?? selected.name}
        hour={hour} detailHours={detailHours} chart={{ ...chart, within }} />
    </> : consumer !== null && measure === 'burn' ? <>
      <button type="button" className="button small quiet" onClick={() => setConsumer(null)}>All fuel sources and consumers</button>
      <strong>{consumerName}</strong>
      {!basis ? <p className="drill-empty">Breakdown unavailable: the applied updates of this day are not known.</p>
        : awards.error ? <WidgetState kind="error" what="recipients" failure={awards.error} onRetry={awards.retry} />
          : !recipients || consumerTotal === null ? <WidgetState kind="loading" label="Loading actual fuel recipients…" read={at} />
            : <DrillParts virtual active={chart.active} positiveBars signedShares parts={recipientParts}
              total={Number(consumerTotal) / Number(gain)} format={String} unattributed={unattributed}
              totalRow={{ label: consumerName, value: formatFuelPart(consumerTotal, gain, digits) }}
              onPick={key => { const part: FuelAwardPart | undefined = listed.find(row => row.key === key); if (part && basis) onSelection(selectFuelRecipient(campaign, basis, part)); }} />}
    </> : <>
      {!reply ? (at.error ? null : <WidgetState kind="loading" label="Loading the day's fuel…" read={at} />)
        : !reply.read ? <p className="drill-empty">{reply.missing ?? 'No daily accounting was recorded for this moment.'}</p>
          : rows && hour !== null || rows && follows ? <DrillParts positiveBars signedShares active={chart.active} parts={rows.parts} live={follows ? live : undefined}
            total={rows.total} format={String} totalRow={{ label: measure === 'gain' ? 'Fuel gained (own production)' : 'Fuel burned', value: rows.totalText }}
            onPick={id => { const part = rows.values.parts.find(row => row.id === id); if (part?.consumer !== undefined) setConsumer(part.consumer); }} />
            : <p className="drill-empty">No daily accounting was recorded for this moment.</p>}
      {rows?.values.reason && <p className="drill-empty">Source breakdown unavailable: {rows.values.reason}</p>}
      {rows?.values.parts.filter(part => part.reason).map(part => <p key={part.id} className="drill-empty">{part.label}: unavailable ({part.reason}).</p>)}
      {rows?.values.transfer && reply && <DrillParts positiveBars showShares={false} parts={[{ id: rows.values.transfer.id, label: rows.values.transfer.label,
        raw: rows.values.transfer.raw, value: Number(rows.values.transfer.raw) / reply.quantityScale,
        text: formatFuelPart(rows.values.transfer.raw, BigInt(reply.quantityScale), digits), color: color('incoming'), muted: rows.values.transfer.muted }]} format={String} />}
      <BreakdownChart {...chartProps} series={own.map(part => part.series)} summaries={{ parts: own.map(part => part.summary!) }}
        label={`${tag}: ${measure === 'gain' ? 'own fuel production' : 'fuel burned'} over time`} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} />
      {incoming.length > 0 && <BreakdownChart {...chartProps} series={incoming.map(part => part.series)} summaries={{ parts: incoming.map(part => part.summary!) }}
        label={`${tag}: incoming lend-lease fuel over time`} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} />}
    </>}
  </div>;
}

function RecipientHistory({ runtime, gate, track, basis, recipientKey, name, hour, detailHours, chart }: {
  runtime: WidgetRuntime; gate: Gate; track: BoundTrack; basis: FuelBasis; recipientKey: string; name: string | null; hour: number | null; detailHours: number; chart: HistoryChart;
}) {
  const tokens = useChartTokens();
  const end = Math.max(chart.extent[0], Math.min(chart.extent[1], hour ?? chart.extent[1]));
  const start = Math.max(chart.extent[0], end - detailHours + 1);
  const [visible, onViewChange] = useDetailView(start, end, detailHours);
  const wanted = detailWindows(chart.extent, visible, hour, detailHours);
  const basisAt = gate.recorded(basis.hour);
  const windows = useProductWindows<FuelAwardHistoryReply>(runtime.client, basisAt === null ? null : JSON.stringify([track.source.id, track.source.revision, basis, recipientKey]),
    window => (productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'country-fuel-award-history',
      params: { branchId: basis.branchId, country: countryFromKey(basis.tag), hour: basisAt!, window, recipientKey } })),
    wanted, chart.active, validateFuelAwardHistory, track.source);
  const runs = useMemo(() => joinedRuns([...windows.data.values()].map(reply => reply.runs)), [windows.data]);
  const amount = (at: number) => {
    if (chart.within && !runAt(chart.within, at)) return 'Outside the selected group membership';
    const run = runAt(runs, at);
    return run?.amountRaw == null ? run?.reason ?? 'History unavailable' : formatFuelPart(BigInt(run.amountRaw), 100000n);
  };
  const current = hour === null ? null : runAt(runs, hour);
  const series: BreakdownSeries[] = [{ id: recipientKey, label: name ?? 'Other', color: tokens.accent, amount, missingAt: amount,
    runs: runs.flatMap((run, i) => run.amountRaw === null ? [] : [{ firstHour: run.firstHour, lastHour: run.lastHour, value: Math.max(0, Number(run.amountRaw) / 100000),
      breakBefore: run.startsSegment || i === 0 || runs[i - 1].amountRaw === null || runs[i - 1].lastHour + 1 !== run.firstHour }]) }];
  return <>
    <DrillParts positiveBars showShares={false} format={String} parts={[{ id: recipientKey, label: name ?? 'Name unavailable',
      value: current?.amountRaw != null ? Number(current.amountRaw) / 100000 : null, raw: current?.amountRaw != null ? BigInt(current.amountRaw) : null,
      text: hour === null ? 'No selected moment' : amount(hour) }]} />
    {chart.active && wanted.some(window => !windows.data.has(window) && !windows.errors.has(window)) && <p className="drill-empty">Loading recipient history…</p>}
    {[...windows.errors.values()].map((reason, i) => <WidgetState key={i} kind="error" what="recipient history" failure={reason} onRetry={windows.retry} />)}
    <BreakdownChart {...chart} series={series} keyHours={runs.map(run => run.firstHour)} selectedHour={hour} onViewChange={onViewChange}
      label={`${name ?? 'Recipient'}: fuel receipts over time`} axes={{ xFrom: start - chart.branchStart, xTo: end - chart.branchStart, yMin: null, yMax: null, log: false }} />
  </>;
}
