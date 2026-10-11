import { useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../components/WidgetState';
import DateTimePicker from '../components/DateTimePicker';
import { useContextMenu } from '../components/ContextMenu';
import DrillParts from '../charts/drill/DrillParts';
import { drawBands, drawPlayhead } from '../charts/draw';
import { jumpItem } from '../charts/jump';
import { lineStyles } from '../charts/palette';
import { countryRaceSegments } from '../charts/race/countryRace';
import { runAt } from '../charts/series';
import { buildStack, stackExtent } from '../charts/stack';
import TimeChart from '../charts/TimeChart';
import type { ChartLayer } from '../charts/TimeChart';
import { useChartTokens } from '../charts/useChartTokens';
import { airLossDrillHistory } from './airLossDrill';
import { formatDateTime } from './calendar';
import { useCountryLooks } from './countryLooks';
import { useCountryLabelsAt } from './countryFlagsAt';
import DatedCountryChip from './DatedCountryChip';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { usePlaybackSpeed } from './playback';
import { AIR_LEDGER_NOTE, LEDGER_GAP_NOTE, airLossParts, ledgerCountryAt, ledgerFormat, ledgerScale, ledgerShare, ledgerSegmentAt, militaryAirTooltip } from './militaryAirLedger';
import type { LedgerIndex } from './militaryAirLedger';

export default function AirLossDrill({ index, runtime, track, tag, identity, countrySlot, headerSelection, from, hour, end, dealt, active, afterPlayhead, onPick, pickOwner }: {
  index: LedgerIndex; runtime: WidgetRuntime; track: BoundTrack; tag: string; identity: string; from: number; hour: number; end: number;
  countrySlot?: HTMLElement | null; headerSelection?: { tag: string; identity: string } | null;
  dealt: boolean; active: boolean; afterPlayhead: boolean; onPick: (hour: number) => void; pickOwner: unknown;
}) {
  const tokens = useChartTokens(), hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const menu = useContextMenu('Aircraft loss history actions');
  const row = hour >= from && hour <= end ? ledgerCountryAt(index, tag, from, hour, identity) : null;
  const model = useMemo(() => airLossDrillHistory(index, identity, from, end, dealt), [index, identity, from, end, dealt]);
  const styles = useMemo(() => lineStyles(model.map((part, preferred) => ({ key: part.id, preferred })), tokens.categories), [model, tokens.categories]);
  const selectionOwner = useMemo(() => ({ pickOwner, client: runtime.client, identity, dealt }), [pickOwner, runtime.client, identity, dealt]);
  const ownerRef = useRef({ owner: selectionOwner, active });
  ownerRef.current = { owner: selectionOwner, active };
  const [selected, setSelected] = useState<{ owner: unknown; id: string } | null>(null);
  const selectedId = selected?.owner === selectionOwner && model.some(part => part.id === selected.id) ? selected.id : null;
  const shown = useMemo(() => selectedId == null ? model : model.filter(part => part.id === selectedId), [model, selectedId]);
  const stack = useMemo(() => buildStack(shown.map(part => part.runs.map(run => ({ ...run, value: Math.max(0, run.value) })))), [shown]);
  const until = afterPlayhead ? end : Math.min(hour, end);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of countryRaceSegments(stack, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]), hoursPerPixel, true))
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, shown.map(part => styles.get(part.id)!.color));
    },
    extent: range => stackExtent(stack, range[0], Math.min(range[1], until)),
  }], [shown, styles, stack, until]);
  const segmentAt = useMemo(() => ledgerSegmentAt(index), [index]);
  const tags = useMemo(() => row ? [row.tag] : [], [row?.tag]);
  const dates = useMemo(() => new Map(tags.map(tag => [tag, hour])), [tags, hour]);
  const looks = useCountryLooks(runtime.client, track, tags, hour, active, segmentAt, dates, true);
  const headerRow = countrySlot && headerSelection && headerSelection.identity === identity && row
    && index.byTag.get(headerSelection.tag)?.some(series => series.country.identity === identity)
    && index.series.filter(series => series.country.identity === identity && runAt(series.country.present, hour)).length === 1
    && index.byTag.get(row.tag)?.filter(series => runAt(series.country.present, hour)).length === 1 ? row : null;
  const wanted = useMemo(() => countrySlot && headerRow ? [{ tag: headerRow.tag, hour }] : [], [countrySlot, headerRow?.tag, hour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active && !!countrySlot);
  const header = countrySlot && createPortal(<DatedCountryChip label={headerRow ? labels.labelAt(hour)(headerRow.tag) : null} hour={hour}
    reason={!headerSelection || !headerRow ? 'Selected country unavailable.' : null} />, countrySlot);
  const historyOwner = useMemo(() => ({ selectionOwner, selectedId }), [selectionOwner, selectedId]);
  const values = row ? airLossParts(row, from, hour) : null, total = row ? dealt ? row.dealt : row.taken : null;
  const parts = model.map(part => {
    const raw = values ? values.causes.get(part.cause)?.[dealt ? 'dealt' : 'taken'] ?? 0n : null;
    return { id: part.id, label: part.label, color: styles.get(part.id)!.color, raw, value: raw == null ? null : Number(raw) / ledgerScale('airLosses'),
      text: ledgerFormat(raw, 'airLosses'), shareText: raw == null || total == null ? undefined : ledgerShare(raw, total) };
  });
  const pickDay = (at: number) => {
    if (!ownerRef.current.active || ownerRef.current.owner !== selectionOwner) return;
    at = Math.floor(at);
    if (at >= from && at <= end) onPick(at);
  };
  return <div className="military-air-ledger air-loss-drill">
    {header}
    <p className="chart-line-key">{row ? looks.name(row.tag, hour) : formatDateTime(hour)} · {dealt ? 'Enemy losses' : 'Own losses'}</p>
    {!row && <>
      <WidgetState kind="unavailable" message="This country or aircraft-loss interval was not recorded at this moment." />
      <DateTimePicker label="Choose breakdown day" value={hour} min={from} max={end} otherValue={null} disabled={!active} describedBy="" onChange={pickDay} />
    </>}
    <DrillParts parts={parts} virtual active={active} positiveBars signedShares showShares={row?.complete ?? false} format={String}
      onPick={id => { if (ownerRef.current.active && ownerRef.current.owner === selectionOwner && model.some(part => part.id === id))
        setSelected(before => before?.owner === selectionOwner && before.id === id ? null : { owner: selectionOwner, id }); }}
      totalRow={{ label: dealt ? 'Enemy losses' : 'Own losses', value: ledgerFormat(total, 'airLosses') }} />
    <p className="chart-line-key">{selectedId == null ? 'All causes' : <>
      History: {model.find(part => part.id === selectedId)?.label}. <button type="button" className="button small"
        onClick={() => { if (ownerRef.current.active && ownerRef.current.owner === selectionOwner) setSelected(null); }}>Show all causes</button>
    </>}</p>
    <TimeChart label="Aircraft loss causes over time" extent={[from, end]} playhead={hour} playback="cursor" afterPlayhead={afterPlayhead}
      hoursPerSecond={hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} branchStart={track.branch.firstHour}
      active={active} layers={layers} revision={stack} pickOwner={historyOwner} overlay={(ctx, frame, chartTokens) => drawPlayhead(ctx, frame, chartTokens, hour)}
      keyHours={[...new Set(shown.flatMap(part => part.runs.map(run => run.firstHour)))].sort((a, b) => a - b)}
      onPick={pickDay} onContextMenu={(at, x, y) => { if (ownerRef.current.active && ownerRef.current.owner === selectionOwner) menu.open(x, y, [jumpItem(runtime, track.id, at)]); }}
      tooltip={at => {
        at = Math.floor(at);
        if (at < from || at > until) return null;
        const current = ledgerCountryAt(index, tag, from, at, identity);
        if (!current) return null;
        return militaryAirTooltip(current, from, 'airLosses', formatDateTime(at));
      }} />
    <p className="chart-line-key">Areas show positive amounts; correction rows and totals retain their signs. {AIR_LEDGER_NOTE} {row?.complete ? '' : LEDGER_GAP_NOTE}</p>
    {menu.menu}
  </div>;
}
