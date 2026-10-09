import { useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import DrillParts from '../../charts/drill/DrillParts';
import { runAt } from '../../charts/series';
import type { Span } from '../../charts/series';
import { useChartTokens } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { isMember } from '../../dashboard/groups';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import { useProductWindows } from '../../dashboard/productWindows';
import type { FuelAwardHistoryReply, FuelAwardPart, FuelAwardRun, FuelAwardsReply, FuelReply } from '../../generated/dashboard';
import { formatFuelPart, fuelAwardNumbers, fuelHistoryRow, fuelParts, fuelPartShare, fuelRecipientAmount, validateFuelAwardHistory, validateFuelAwardsReply } from './fuelData';
import { fuelCountryHistory, fuelCountryRead } from './fuelCountry';
import { fuelRecipientBasis, fuelRecipientForReply, sameFuelActor, selectFuelRecipient } from './fuelDrill';
import type { FuelRecipientSelection } from './fuelDrill';

type HistoryChart = Omit<Parameters<typeof BreakdownChart>[0], 'series' | 'axes' | 'label'>;

export default function FuelBreakdown({ reply, history, actorSpans, identitySpans, measure, hour, tokens, within, runtime, track, selection: retainedSelection, onSelection, ...chart }: {
  reply: FuelReply; measure: 'gain' | 'burn'; hour: number | null; tokens: ReturnType<typeof useChartTokens>;
  history?: readonly FuelReply[]; actorSpans?: ReadonlyMap<string, readonly Span[]>; identitySpans?: ReadonlyMap<string, readonly Span[]>;
  runtime: WidgetRuntime; track: BoundTrack; selection: FuelRecipientSelection | null;
  onSelection: (selection: FuelRecipientSelection | null) => void;
} & HistoryChart) {
  const selection = fuelRecipientForReply(retainedSelection, reply, track.source.metadata.campaignKey, identitySpans ?? actorSpans);
  const [pickedConsumer, setConsumer] = useState<number | null>(null);
  useEffect(() => { setConsumer(null); }, [chart.pickOwner]);
  const consumer = selection?.consumer ?? pickedConsumer;
  const read = hour !== null && isMember(within ?? undefined, hour) ? runAt(reply.reads, hour) : null;
  const values = read && fuelParts(reply, read, measure);
  const sources = useMemo(() => history ?? [reply], [history, reply]);
  const histories = useMemo(() => fuelCountryHistory(sources, actorSpans, measure), [sources, actorSpans, measure]);
  const series = useMemo(() => histories.map((part, i): BreakdownSeries => ({ id: part.id, label: part.label,
    color: tokens.categories[i % tokens.categories.length], runs: part.runs,
    labelAt: at => {
      const source = fuelCountryRead(sources, actorSpans, at), value = source && fuelParts(source.reply, source.read, measure);
      return fuelHistoryRow(value ?? null, part.id, part.transfer)?.label ?? 'Other / not identified';
    },
    amount: at => {
      const source = fuelCountryRead(sources, actorSpans, at), value = source && fuelParts(source.reply, source.read, measure);
      const row = fuelHistoryRow(value ?? null, part.id, part.transfer);
      return row && value && source ? formatFuelPart(row.raw, part.transfer ? BigInt(source.reply.quantityScale) : value.scale) : 'Not recorded';
    },
  })), [histories, sources, actorSpans, measure, tokens.categories]);
  const color = (id: string) => series.find(part => part.id === id)?.color;
  const basis = fuelRecipientBasis(reply, read, retainedSelection, track.source.metadata.campaignKey, identitySpans ?? actorSpans);
  const request = basis && measure === 'burn' && consumer !== null ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-fuel-awards', params: basis } : null;
  const recipients = useProduct<FuelAwardsReply>(runtime.client, request, chart.active, validateFuelAwardsReply);
  const retained = useMemo(() => new NumericReply<FuelAwardsReply>(), [runtime.client]);
  const numbers = retained.update(JSON.stringify(request), recipients.data, fuelAwardNumbers);
  const data = recipients.data ?? numbers;
  const contentPending = !!numbers && !recipients.data && !recipients.error;
  const awards = data && basis && data.hour === basis.hour && data.branchId === basis.branchId
    && data.tag === basis.tag ? data.awards : null;
  const selected = selection && awards?.recipients.find(part => part.key === selection.key);
  const matching = !selection || sameFuelActor(selection, read);
  const recipientRows = useMemo(() => (awards?.recipients ?? []).filter(part => part.kind === consumer), [awards, consumer]);
  const recipientTotal = awards && consumer !== null ? BigInt(awards.totals.receivedRaw[consumer]) : null;
  const assigned = useMemo(() => recipientRows.reduce((sum, part) => sum + BigInt(part.amountRaw), 0n), [recipientRows]);
  const recipientParts = useMemo(() => recipientRows.map(part => {
    const raw = BigInt(part.amountRaw);
    return { id: part.key, label: part.recordedName ?? `Other / not identified (${part.reference})`, raw,
      value: Number(raw) / reply.gainScale, text: formatFuelPart(raw, BigInt(reply.gainScale)),
      shareText: fuelPartShare(raw, recipientTotal!), muted: part.recordedName === null };
  }), [recipientRows, reply.gainScale, recipientTotal]);
  const unattributed = useMemo(() => recipientTotal !== null && assigned !== recipientTotal ? { label: 'Other / not identified',
    raw: recipientTotal - assigned, value: Number(recipientTotal - assigned) / reply.gainScale,
    text: formatFuelPart(recipientTotal - assigned, BigInt(reply.gainScale)), shareText: fuelPartShare(recipientTotal - assigned, recipientTotal) }
    : undefined, [recipientTotal, assigned, reply.gainScale]);
  return <>
    {contentPending && <p className="drill-empty">Recipient names are updating from the recorded content.</p>}
    {retainedSelection && !selection && <p className="drill-empty">The saved recipient belongs to another country, campaign or branch.</p>}
    {selection ? <>
      <button type="button" className="button small quiet" onClick={() => { setConsumer(selection.consumer); onSelection(null); }}>All consumer recipients</button>
      {selected && <RecipientFacts part={selected} reply={reply} />}
      {!matching && <p className="drill-empty">This moment belongs to another or unrecorded native country identity.</p>}
      {selected && <FuelRecipientHistory key={JSON.stringify(selection)} runtime={runtime} track={track} reply={reply} selection={selection} part={selected}
        hour={hour} within={within} {...chart} />}
    </> : consumer !== null && measure === 'burn' ? <>
      <button type="button" className="button small quiet" onClick={() => setConsumer(null)}>All fuel sources and consumers</button>
      <strong>{reply.consumerNames[consumer] ?? 'Other / not identified'}</strong>
      {!basis ? <p className="drill-empty">{read?.awardsReason ?? 'Applied recipient accounting was not recorded for this moment.'}</p>
        : recipients.error ? <WidgetState kind="error" message={recipients.error} onRetry={recipients.retry} />
          : !awards ? <WidgetState kind="loading" label="Loading actual fuel recipients..." /> : <>
            <DrillParts virtual active={chart.active} positiveBars signedShares parts={recipientParts}
              total={Number(recipientTotal) / reply.gainScale} format={String} unattributed={unattributed}
              totalRow={{ label: reply.consumerNames[consumer] ?? 'Consumer receipts', value: formatFuelPart(recipientTotal!, BigInt(reply.gainScale)) }}
              onPick={key => {
                const part = recipientRows.find(row => row.key === key);
                if (read && part) onSelection(selectFuelRecipient(reply, read, part, track.source.metadata.campaignKey));
              }} />
            <p className="drill-empty">Select any exact signed row for its recorded identity, name refusal and history. Receipts are country fuel awarded to native recipients; air containers do not identify wing receipts.</p>
          </>}
    </> : <>
      {values && hour !== null ? <DrillParts positiveBars signedShares active={chart.active} parts={values.parts.map(part => ({ id: part.id,
        label: part.label, color: color(part.id), raw: part.raw, value: Number(part.raw) / Number(values.scale),
        text: formatFuelPart(part.raw, values.scale), shareText: fuelPartShare(part.raw, values.total), muted: part.muted,
        selectable: measure === 'burn' && part.consumer !== undefined && !!read?.day?.awards }))}
        total={Number(values.total) / Number(values.scale)} format={String}
        totalRow={{ label: measure === 'gain' ? 'Fuel gained (own production)' : 'Fuel burned', value: formatFuelPart(values.total, values.scale) }}
        onPick={id => { const part = values.parts.find(row => row.id === id); if (part?.consumer !== undefined) setConsumer(part.consumer); }} />
        : <p className="drill-empty">No daily accounting was recorded for this moment.</p>}
      {values?.reason && <p className="drill-empty">Applied source accounting unavailable: {values.reason}. The recorded ring totals remain available.</p>}
      {values?.parts.filter(part => part.reason).map(part => <p key={part.id} className="drill-empty">{part.reason}</p>)}
      {values?.transfer && <>
        <p className="drill-empty">Incoming lend-lease is separate from own production and its shares.</p>
        <DrillParts positiveBars showShares={false} parts={[{ id: values.transfer.id, label: values.transfer.label,
          raw: values.transfer.raw, value: Number(values.transfer.raw) / reply.quantityScale,
          text: formatFuelPart(values.transfer.raw, BigInt(reply.quantityScale)), color: color(values.transfer.id), muted: values.transfer.muted }]} format={String} />
        {values.transfer.reason && <p className="drill-empty">{values.transfer.reason}</p>}
      </>}
      {read && <p className="drill-empty">Read {formatDateTime(read.firstHour)}; accounting covers the {reply.dayHours} hours before that read. Signed corrections retain their exact rows; positive amounts draw the stack.</p>}
      <BreakdownChart {...chart} within={within} series={series.filter(part => part.id !== 'incoming')} selectedHour={hour}
        label={`${reply.tag}: ${measure === 'gain' ? 'own fuel production' : 'fuel burned'} over time`}
        axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} />
      {series.some(part => part.id === 'incoming') && <BreakdownChart {...chart} within={within} series={series.filter(part => part.id === 'incoming')}
        selectedHour={hour} label={`${reply.tag}: incoming lend-lease fuel over time`}
        axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} />}
    </>}
    {selection && recipients.error && <WidgetState kind="error" message={recipients.error} onRetry={recipients.retry} />}
    {selection && !selected && !recipients.error && <p className="drill-empty">{awards ? 'The selected native recipient is unavailable at its supplying read.' : 'Loading the selected native recipient...'}</p>}
    {recipients.data?.warnings.map((note, i) => <p key={i} className="drill-empty">{note}</p>)}
  </>;
}

function RecipientFacts({ part, reply }: { part: FuelAwardPart; reply: FuelReply }) {
  return <>
    <strong>{part.recordedName ?? `Other / not identified (${part.reference})`}</strong>
    <p className="drill-empty">{reply.consumerNames[part.kind] ?? 'Other / not identified'} · native reference {part.reference} · owner {part.owner} · host {part.host}.
      Observed receipts {formatDateTime(part.firstHour)}–{formatDateTime(part.lastHour)}.</p>
    {part.nameReason && <p className="drill-empty">{part.nameReason}</p>}
  </>;
}

function FuelRecipientHistory({ runtime, track, reply, selection, part, hour, within, ...chart }: {
  runtime: WidgetRuntime; track: BoundTrack; reply: FuelReply; selection: FuelRecipientSelection; part: FuelAwardPart;
  hour: number | null;
} & HistoryChart) {
  const end = Math.max(chart.extent[0], Math.min(chart.extent[1], hour ?? chart.extent[1]));
  const start = Math.max(chart.extent[0], end - reply.detailHours + 1);
  const [visible, setVisible] = useState<readonly [number, number]>([start, end]);
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
  const series: BreakdownSeries[] = [{ id: selection.key, label: part.recordedName ?? 'Other / not identified', color: useChartTokens().accent,
    runs: runs.flatMap((row, i) => row.amountRaw === null ? [] : [{ firstHour: row.firstHour, lastHour: row.lastHour,
      value: Math.max(0, Number(row.amountRaw) / reply.gainScale),
      breakBefore: row.startsSegment || i === 0 || runs[i - 1].amountRaw === null || runs[i - 1].lastHour + 1 !== row.firstHour }]), amount, missingAt: amount }];
  return <>
    <DrillParts positiveBars showShares={false} parts={[{ id: selection.key, label: part.recordedName ?? 'Other / not identified',
      ...current }]} format={String} />
    {[...windows.errors.values()].map((reason, i) => <WidgetState key={i} kind="error" message={reason} onRetry={windows.retry} />)}
    {[...windows.data.values()].flatMap(detail => detail.warnings).map((note, i) => <p key={i} className="drill-empty">{note}</p>)}
    <BreakdownChart {...chart} within={within} series={series} keyHours={runs.map(row => row.firstHour)} selectedHour={hour}
      onViewChange={range => setVisible(previous => previous[0] === range[0] && previous[1] === range[1] ? previous : range)}
      label={`${part.recordedName ?? 'Native recipient'}: actual fuel receipts over time`}
      axes={{ xFrom: start - chart.branchStart, xTo: end - chart.branchStart, yMin: null, yMax: null, log: false }} />
  </>;
}
