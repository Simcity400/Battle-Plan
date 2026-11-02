import { asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import DrillParts from '../../charts/drill/DrillParts';
import { runAt } from '../../charts/series';
import type { Span } from '../../charts/series';
import { useChartTokens } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { isMember } from '../../dashboard/groups';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import { useProductWindows } from '../../dashboard/productWindows';
import type { FuelAwardHistoryReply, FuelAwardPart, FuelAwardRun, FuelAwardsReply, FuelReply } from '../../generated/dashboard';
import { formatFuelPart, fuelAwardNumbers, fuelHistoryRow, fuelParts, fuelPartShare, fuelRecipientAmount, validateFuelAwardHistory, validateFuelAwardsReply } from './fuelData';
import { usePreparedFuel, fuelSummaryRuns, useFuelSeriesParity } from './preparedFuel';
import { fuelCountryHistory, fuelCountryRead } from './fuelCountry';
import { fuelRecipientBasis, fuelRecipientForReply, sameFuelActor, selectFuelRecipient } from './fuelDrill';
import type { FuelRecipientSelection } from './fuelDrill';

type HistoryChart = Omit<Parameters<typeof BreakdownChart>[0], 'series' | 'axes' | 'label'>;

/** A detail history chart's drawn range, kept while it reaches the same detail
 * windows, so zoom and playback easing re-render only when it reaches another. */
export function useDetailView(start: number, end: number, detailHours: number) {
  const [visible, setVisible] = useState<readonly [number, number]>([start, end]);
  const onViewChange = useCallback((x: readonly [number, number]) => setVisible(before => {
    const block = (hour: number) => Math.floor(hour / detailHours);
    return block(before[0]) === block(x[0]) && block(before[1]) === block(x[1]) ? before : x;
  }), [detailHours]);
  return [visible, onViewChange] as const;
}

/** Country amounts are written to `digits` decimals; a single recipient's receipts
 * stay exact, since a fraction of one fuel would read as 0 or 1. */
export default function FuelBreakdown({ reply: suppliedReply, history: suppliedHistory, actorSpans, identitySpans, measure, hour, tokens, within, runtime, track, selection: retainedSelection, onSelection, digits, ...chart }: {
  reply: FuelReply; measure: 'gain' | 'burn'; hour: number | null; tokens: ReturnType<typeof useChartTokens>; digits: number;
  history?: readonly FuelReply[]; actorSpans?: ReadonlyMap<string, readonly Span[]>; identitySpans?: ReadonlyMap<string, readonly Span[]>;
  runtime: WidgetRuntime; track: BoundTrack; selection: FuelRecipientSelection | null;
  onSelection: (selection: FuelRecipientSelection | null) => void;
} & HistoryChart) {
  const prepared = usePreparedFuel(runtime.client, track, [...((actorSpans ?? identitySpans)?.keys() ?? [suppliedReply.tag])], Math.floor(hour ?? chart.extent[1]), chart.active,
    measure === 'gain' ? ['gain-0','gain-1','gain-2','gain-3','unidentified','incoming'] : ['consumer-0','consumer-1','consumer-2','outgoing','flat','other','conversion'], actorSpans);
  const reply = prepared.enabled ? prepared.data?.find(reply=>reply.tag===suppliedReply.tag) ?? suppliedReply : suppliedReply;
  const history = prepared.enabled ? prepared.data ?? suppliedHistory : suppliedHistory;
  const parity = useRef<HTMLDivElement>(null);
  useFuelSeriesParity(parity, runtime.client, track, [...((actorSpans ?? identitySpans)?.keys() ?? [reply.tag])], chart.extent, actorSpans, 'Fuel accounting values', history ?? [reply]);
  const selection = fuelRecipientForReply(retainedSelection, reply, track.source.metadata.campaignKey, identitySpans ?? actorSpans);
  const [pickedConsumer, setConsumer] = useState<number | null>(null);
  useEffect(() => { setConsumer(null); }, [chart.pickOwner]);
  const consumer = selection?.consumer ?? pickedConsumer;
  const read = hour !== null && isMember(within ?? undefined, hour) ? runAt(reply.reads, hour) : null;
  const values = read && fuelParts(reply, read, measure, reply, digits);
  const sources = useMemo(() => history ?? [reply], [history, reply]);
  const histories = useMemo(() => {
    const legacy = fuelCountryHistory(sources, actorSpans, measure, digits);
    if (!prepared.enabled) return legacy;
    const parts = new Map<string, typeof legacy[number]>();
    for (const line of prepared.summaries?.series ?? []) {
      if (!line.summary.buckets.some(bucket=>bucket!==null) && !line.summary.runs?.length) continue;
      const scale = line.measure === 'incoming' ? reply.quantityScale : measure === 'gain' ? reply.gainScale : reply.quantityScale * reply.gainScale;
      const named = legacy.find(part=>part.id===line.measure);
      const part = parts.get(line.measure) ?? {id:line.measure,label:named?.label ?? 'Other',runs:[],transfer:line.measure==='incoming'};
      part.runs.push(...fuelSummaryRuns(line.summary,scale));parts.set(line.measure,part);
    }
    return [...parts.values()].map(part=>({...part,runs:part.runs.sort((a,b)=>a.firstHour-b.firstHour)}));
  }, [sources, actorSpans, measure, digits, prepared.summaries, reply.gainScale, reply.quantityScale]);
  const series = useMemo(() => histories.map((part, i): BreakdownSeries => ({ id: part.id, label: part.label,
    color: tokens.categories[i % tokens.categories.length], runs: part.runs,
    labelAt: at => {
      const source = fuelCountryRead(sources, actorSpans, at), value = source && fuelParts(source.reply, source.read, measure, source.reply, digits);
      return fuelHistoryRow(value ?? null, part.id, part.transfer)?.label ?? 'Other';
    },
    amount: at => {
      const source = fuelCountryRead(sources, actorSpans, at), value = source && fuelParts(source.reply, source.read, measure, source.reply, digits);
      const row = fuelHistoryRow(value ?? null, part.id, part.transfer);
      return row && value && source ? formatFuelPart(row.raw, part.transfer ? BigInt(source.reply.quantityScale) : value.scale, digits) : 'Not recorded';
    },
  })), [histories, sources, actorSpans, measure, digits, tokens.categories]);
  const color = (id: string) => series.find(part => part.id === id)?.color;
  const basis = fuelRecipientBasis(reply, read, retainedSelection, track.source.metadata.campaignKey, identitySpans ?? actorSpans);
  const request = basis && measure === 'burn' && consumer !== null ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-fuel-awards', params: basis } : null;
  const recipients = useProduct<FuelAwardsReply>(runtime.client, request, chart.active, validateFuelAwardsReply);
  const retained = useMemo(() => new NumericReply<FuelAwardsReply>(), [runtime.client]);
  const numbers = retained.update(JSON.stringify(request), recipients.data, fuelAwardNumbers);
  const data = recipients.data ?? numbers;
  const awards = data && basis && data.hour === basis.hour && data.branchId === basis.branchId
    && data.tag === basis.tag ? data.awards : null;
  const selected = selection && awards?.recipients.find(part => part.key === selection.key);
  const matching = !selection || sameFuelActor(selection, read);
  const recipientRows = useMemo(() => (awards?.recipients ?? []).filter(part => part.kind === consumer), [awards, consumer]);
  const recipientTotal = awards && consumer !== null ? BigInt(awards.totals.receivedRaw[consumer]) : null;
  const assigned = useMemo(() => recipientRows.reduce((sum, part) => sum + BigInt(part.amountRaw), 0n), [recipientRows]);
  const recipientParts = useMemo(() => recipientRows.map(part => {
    const raw = BigInt(part.amountRaw);
    return { id: part.key, label: part.recordedName ?? 'Name unavailable', raw,
      value: Number(raw) / reply.gainScale, text: formatFuelPart(raw, BigInt(reply.gainScale)),
      shareText: fuelPartShare(raw, recipientTotal!), muted: part.recordedName === null };
  }), [recipientRows, reply.gainScale, recipientTotal]);
  const unattributed = useMemo(() => recipientTotal !== null && assigned !== recipientTotal ? { label: 'Other',
    raw: recipientTotal - assigned, value: Number(recipientTotal - assigned) / reply.gainScale,
    text: formatFuelPart(recipientTotal - assigned, BigInt(reply.gainScale)), shareText: fuelPartShare(recipientTotal - assigned, recipientTotal) }
    : undefined, [recipientTotal, assigned, reply.gainScale]);
  return <div ref={parity}>
    {/* The accounting's own reads: a failure says so, keeping any history already drawn. */}
    {prepared.enabled && prepared.error && <WidgetState kind="error" what="fuel accounting" failure={asFailure(prepared.error)} onRetry={prepared.retry} />}
    {retainedSelection && !selection && <p className="drill-empty">Choose a recipient again.</p>}
    {selection ? <>
      <button type="button" className="button small quiet" onClick={() => { setConsumer(selection.consumer); onSelection(null); }}>All consumer recipients</button>
      {selected && <RecipientFacts part={selected} reply={reply} />}
      {!matching && <p className="drill-empty">Country unavailable at this time.</p>}
      {selected && <FuelRecipientHistory key={JSON.stringify(selection)} runtime={runtime} track={track} reply={reply} selection={selection} part={selected}
        hour={hour} within={within} {...chart} />}
    </> : consumer !== null && measure === 'burn' ? <>
      <button type="button" className="button small quiet" onClick={() => setConsumer(null)}>All fuel sources and consumers</button>
      <strong>{reply.consumerNames[consumer] ?? 'Consumer'}</strong>
      {!basis ? <p className="drill-empty">Breakdown unavailable.</p>
        : recipients.error ? <WidgetState kind="error" what="recipients" failure={recipients.error} onRetry={recipients.retry} />
          : !awards ? <WidgetState kind="loading" label="Loading actual fuel recipients..." /> : <>
            <DrillParts virtual active={chart.active} positiveBars signedShares parts={recipientParts}
              total={Number(recipientTotal) / reply.gainScale} format={String} unattributed={unattributed}
              totalRow={{ label: reply.consumerNames[consumer] ?? 'Consumer receipts', value: formatFuelPart(recipientTotal!, BigInt(reply.gainScale), digits) }}
              onPick={key => {
                const part = recipientRows.find(row => row.key === key);
                if (read && part) onSelection(selectFuelRecipient(reply, read, part, track.source.metadata.campaignKey));
              }} />
          </>}
    </> : <>
      {values && hour !== null ? <DrillParts positiveBars signedShares active={chart.active} parts={values.parts.map(part => ({ id: part.id,
        label: part.label, color: color(part.id), raw: part.raw, value: Number(part.raw) / Number(values.scale),
        text: formatFuelPart(part.raw, values.scale, digits), shareText: fuelPartShare(part.raw, values.total), muted: part.muted,
        selectable: measure === 'burn' && part.consumer !== undefined && !!read?.day?.awards }))}
        total={Number(values.total) / Number(values.scale)} format={String}
        totalRow={{ label: measure === 'gain' ? 'Fuel gained (own production)' : 'Fuel burned', value: formatFuelPart(values.total, values.scale, digits) }}
        onPick={id => { const part = values.parts.find(row => row.id === id); if (part?.consumer !== undefined) setConsumer(part.consumer); }} />
        : <p className="drill-empty">No daily accounting was recorded for this moment.</p>}
      {values?.reason && <p className="drill-empty">Source breakdown unavailable.</p>}
      {values?.parts.filter(part => part.reason).map(part => <p key={part.id} className="drill-empty">{part.label}: Unavailable.</p>)}
      {values?.transfer && <>
        <DrillParts positiveBars showShares={false} parts={[{ id: values.transfer.id, label: values.transfer.label,
          raw: values.transfer.raw, value: Number(values.transfer.raw) / reply.quantityScale,
          text: formatFuelPart(values.transfer.raw, BigInt(reply.quantityScale), digits), color: color(values.transfer.id), muted: values.transfer.muted }]} format={String} />
        {values.transfer.reason && <p className="drill-empty">Incoming lend-lease unavailable.</p>}
      </>}
      <BreakdownChart {...chart} onSummaryView={prepared.enabled ? prepared.onSummaryView : undefined} onHover={prepared.enabled ? prepared.onHover : undefined} onNavigateHour={prepared.enabled ? prepared.onNavigateHour : undefined} keyboardHour={prepared.keyboardHour} keyboardRevision={prepared.keyboardRevision} within={within} series={series.filter(part => part.id !== 'incoming')} selectedHour={hour}
        label={`${reply.tag}: ${measure === 'gain' ? 'own fuel production' : 'fuel burned'} over time`}
        axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} />
      {series.some(part => part.id === 'incoming') && <BreakdownChart {...chart} onSummaryView={prepared.enabled ? prepared.onSummaryView : undefined} onHover={prepared.enabled ? prepared.onHover : undefined} onNavigateHour={prepared.enabled ? prepared.onNavigateHour : undefined} keyboardHour={prepared.keyboardHour} keyboardRevision={prepared.keyboardRevision} within={within} series={series.filter(part => part.id === 'incoming')}
        selectedHour={hour} label={`${reply.tag}: incoming lend-lease fuel over time`}
        axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} />}
    </>}
    {selection && recipients.error && <WidgetState kind="error" what="recipients" failure={recipients.error} onRetry={recipients.retry} />}
    {selection && !selected && !recipients.error && <p className="drill-empty">{awards ? 'Recipient unavailable.' : 'Loading recipient…'}</p>}
  </div>;
}

function RecipientFacts({ part, reply }: { part: FuelAwardPart; reply: FuelReply }) {
  return <>
    <strong>{part.recordedName ?? 'Name unavailable'}</strong>
    <p className="drill-empty">{reply.consumerNames[part.kind] ?? 'Consumer'}</p>
  </>;
}

function FuelRecipientHistory({ runtime, track, reply, selection, part, hour, within, ...chart }: {
  runtime: WidgetRuntime; track: BoundTrack; reply: FuelReply; selection: FuelRecipientSelection; part: FuelAwardPart;
  hour: number | null;
} & HistoryChart) {
  const end = Math.max(chart.extent[0], Math.min(chart.extent[1], hour ?? chart.extent[1]));
  const start = Math.max(chart.extent[0], end - reply.detailHours + 1);
  const [visible, onViewChange] = useDetailView(start, end, reply.detailHours);
  const from = Math.max(chart.extent[0], visible[0]), to = Math.min(chart.extent[1], visible[1]);
  const wanted: number[] = [];
  for (let window = Math.floor(from / reply.detailHours); window <= Math.floor(to / reply.detailHours); window++) wanted.push(window);
  if (hour !== null && hour >= chart.extent[0] && hour <= chart.extent[1]) wanted.push(Math.floor(hour / reply.detailHours));
  const identity = JSON.stringify([track.source.id, track.source.revision, selection]);
  const windows = useProductWindows<FuelAwardHistoryReply>(runtime.client, identity, window => ({ sourceId: track.source.id,
    revision: track.source.revision, product: 'country-fuel-award-history', params: { ...selection.basis, window, recipientKey: selection.key } }),
    [...new Set(wanted)], chart.active, data => {
      const result = validateFuelAwardHistory(data);
      if (result.branchId !== selection.basis.branchId || result.tag !== selection.basis.tag || result.hour !== selection.basis.hour
        || result.country !== selection.basis.country || result.countryInstance !== selection.basis.countryInstance
        || result.continuityStartHour !== selection.continuityStartHour || result.recipientKey !== selection.key
        || result.firstHour !== result.window * reply.detailHours || result.lastHour !== (result.window + 1) * reply.detailHours - 1) {
        throw new Error('The fuel history belongs to another native recipient.');
      }
      return result;
    });
  const runs = useMemo(() => {
    const reads = new Map<number, FuelAwardRun>();
    for (const detail of windows.data.values()) for (const run of detail.runs) {
      const previous = reads.get(run.firstHour);
      if (!previous || run.lastHour > previous.lastHour) reads.set(run.firstHour, run);
    }
    return [...reads.values()].sort((a, b) => a.firstHour - b.firstHour);
  }, [windows.data]);
  const current = fuelRecipientAmount(runs, hour, within, reply.gainScale);
  const amount = (at: number) => fuelRecipientAmount(runs, at, within, reply.gainScale).text;
  const series: BreakdownSeries[] = [{ id: selection.key, label: part.recordedName ?? 'Other', color: useChartTokens().accent,
    runs: runs.flatMap((row, i) => row.amountRaw === null ? [] : [{ firstHour: row.firstHour, lastHour: row.lastHour,
      value: Math.max(0, Number(row.amountRaw) / reply.gainScale),
      breakBefore: row.startsSegment || i === 0 || runs[i - 1].amountRaw === null || runs[i - 1].lastHour + 1 !== row.firstHour }]), amount, missingAt: amount }];
  return <>
    <DrillParts positiveBars showShares={false} parts={[{ id: selection.key, label: part.recordedName ?? 'Name unavailable',
      ...current }]} format={String} />
    {[...windows.errors.values()].map((reason, i) => <WidgetState key={i} kind="error" what="recipient history" failure={reason} onRetry={windows.retry} />)}
    <BreakdownChart {...chart} within={within} series={series} keyHours={runs.map(row => row.firstHour)} selectedHour={hour}
      onViewChange={onViewChange}
      label={`${part.recordedName ?? 'Recipient'}: fuel receipts over time`}
      axes={{ xFrom: start - chart.branchStart, xTo: end - chart.branchStart, yMin: null, yMax: null, log: false }} />
  </>;
}
