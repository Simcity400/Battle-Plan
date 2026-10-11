import { useCallback, useMemo, useState } from 'react';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart } from '../../charts/drill/DrillParts';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { jumpItem } from '../../charts/jump';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import { compactStack, compactStackExtent, forEachCompactStackSegment } from '../../charts/stack';
import { heldStackSegment } from '../../charts/race/countryRace';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { BoundTrack } from '../../dashboard/model';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import { transferAmount, transfersThrough } from './territoryData';
import type { TransferLink, TransferMeasure, TransferOccurrence } from './territoryData';
import { transferHistory, transferPartRaw } from './transferDrillData';
import { transferTipRequest } from './territoryTooltip';

const EMPTY_IDS: readonly string[] = [];
const EMPTY_ROWS: readonly LogRow<TransferOccurrence>[] = [];

/** The complete raw link inventory. Geometry never determines what can be read. */
export default function TransferDrill({ links, measure, day, cutoff, first, last, from, runtime, track, active, onPick, onContextMenu, colorOf }: {
  links: readonly TransferLink[]; measure: TransferMeasure; day: number; cutoff: number; first: number; last: number; from: number;
  runtime: WidgetRuntime; track: BoundTrack; active: boolean; onPick: (hour: number) => void;
  onContextMenu: (hour: number, x: number, y: number) => void; colorOf: (link: TransferLink) => string;
}) {
  const current = useMemo(() => transfersThrough(links, Math.floor(day), Math.floor(day), from), [links, day, from]);
  const byId = useMemo(() => new Map(current.map(link => [link.id, link])), [current]);
  const [visibleIds, setVisibleIds] = useState<readonly string[]>(EMPTY_IDS);
  const [detail, setDetail] = useState<string | null>(null);
  const [visibleRows, setVisibleRows] = useState<readonly LogRow<TransferOccurrence>[]>(EMPTY_ROWS);
  const detailLinks = useMemo(() => detail == null ? current : current.filter(link => link.id === detail), [current, detail]);
  const rows = useMemo(() => detailLinks.flatMap(link => link.states.map((state, index) => ({ key: JSON.stringify([link.id, index, state.state, state.dates]), hour: state.dates.afterHour, item: state })))
    .sort((a, b) => b.hour - a.hour || a.key.localeCompare(b.key)), [detailLinks]);
  const wanted = useMemo(() => {
    const states = visibleRows.map(row => row.item);
    for (const id of visibleIds) { const state = byId.get(id)?.states.at(-1); if (state) states.push(state); }
    return transferTipRequest(states);
  }, [visibleRows, visibleIds, byId]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted.countries, active, Infinity);
  const stateAt = useStateNamesAt(runtime.client, track, wanted.states, active);
  const name = useCallback((link: TransferLink) => {
    const state = link.states.at(-1)!;
    return `${labelAt(state.fromCountry.hour)(state.fromCountry.tag).name} → ${labelAt(state.toCountry.hour)(state.toCountry.tag).name}`;
  }, [labelAt]);
  const total = current.reduce<bigint | null>((sum, link) => {
    const value = transferPartRaw(link, measure);
    return sum == null || value == null ? null : sum + value;
  }, 0n);
  const parts: DrillPart[] = current.map(link => {
    const value = transferPartRaw(link, measure);
    return { id: link.id, label: name(link), raw: value, value: value == null ? null : Number(value), color: colorOf(link),
      text: value == null ? 'Not recorded' : usNumber.format(value),
      shareText: value == null || total == null || total === 0n ? '—' : `${Number(value * 1000n / total) / 10}%` };
  });
  const bands = useMemo(() => transferHistory(links, measure, first, last, from), [links, measure, first, last, from]);
  const stack = useMemo(() => compactStack(bands), [bands]);
  const colors = useMemo(() => links.map(colorOf), [links, colorOf]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      forEachCompactStackSegment(stack, frame.x[0], end, Math.max(1, (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth)), piece => {
        const held = heldStackSegment(piece);
        drawBands(ctx, frame, chartTokens, held.hours, held.bands, colors, held);
      });
    },
    extent: x => compactStackExtent(stack, x[0], Math.min(x[1], cutoff)),
  }], [stack, colors, cutoff]);
  const vp = (state: TransferOccurrence) => state.victoryPointsPending ? state.victoryPointsReadHour == null ? 'VP read hour not recorded'
    : `VP not recorded at this moment; read ${formatDateTime(state.victoryPointsReadHour)}` : state.victoryPoints == null ? 'VP not recorded'
      : `${usNumber.format(BigInt(state.victoryPoints))} VP${state.victoryPointsReadHour == null ? '; read hour not recorded' : `; read ${formatDateTime(state.victoryPointsReadHour)}`}`;
  const describe = (row: LogRow<TransferOccurrence>) => {
    const state = row.item, place = stateAt(row.hour, state.state);
    return `${place.name ?? `State ${state.state}`} ${labelAt(state.fromCountry.hour)(state.fromCountry.tag).name} to ${labelAt(state.toCountry.hour)(state.toCountry.tag).name}; ${vp(state)}; ${state.derived === false ? 'Captured' : state.derived ? 'Derived net change' : 'Provenance not recorded'}${place.missing ? `; ${place.missing}` : ''}`;
  };
  const outside = day < first || day > last;
  return <div className="transfer-drill">
    {outside ? <p className="drill-empty">This time is outside the selected recorded interval.</p> : <div className="transfer-drill-parts">
      <DrillParts parts={parts} format={value => usNumber.format(value)} virtual active={active} positiveBars signedShares
        onVisible={setVisibleIds} onPick={setDetail} totalRow={{ label: 'Total', value: total == null ? 'Not recorded' : usNumber.format(total) }} />
    </div>}
    <div className="transfer-drill-history"><TimeChart label="Transfer parts over time" extent={[first, last]} playhead={cutoff} playback="cursor" afterPlayhead={false}
      branchStart={track.branch.firstHour} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      layers={layers} active={active} revision={[stack, colors]} onPick={onPick} onContextMenu={onContextMenu}
      overlay={(ctx, frame, chartTokens) => { if (day >= frame.x[0] && day <= frame.x[1]) drawPlayhead(ctx, frame, chartTokens, day); }}
      tooltip={hour => { const known = transfersThrough(links, Math.floor(hour), Math.floor(hour), from); return { title: dayLabel(hour), rows: [{ label: measure === 'states' ? 'State occurrences' : 'Victory points', value: transferAmount(known, measure) }], note: 'Click to show this day. Missing VP reads break the stack.' }; }} />
    </div>
    {!outside && <div className="transfer-drill-occurrences">
      <div className="transfer-drill-detail-heading"><strong>{detail == null ? 'Recorded state occurrences' : 'Selected link’s state occurrences'}</strong>
        {detail != null && <button type="button" className="button small quiet" onClick={() => setDetail(null)}>All occurrences</button>}</div>
      <LogList rows={rows} label="Transferred states" density="comfortable" months={false} animate={false} layout="card" rowHeight={() => 96}
        onVisible={setVisibleRows} describe={describe} menu={row => [{ label: 'Show this day', run: () => onPick(row.hour) }, jumpItem(runtime, track.id, row.hour)]}
        renderRow={row => { const state = row.item, place = stateAt(row.hour, state.state); return <div className="transfer-occurrence">
          <strong>{place.name ?? `State ${state.state} · name not recorded`}</strong><span>{labelAt(state.fromCountry.hour)(state.fromCountry.tag).name} → {labelAt(state.toCountry.hour)(state.toCountry.tag).name}</span>
          <span>{state.derived === false ? 'Captured' : state.derived ? 'By' : 'Read through'} {formatDateTime(row.hour)}{state.derived !== false ? `; compared with ${formatDateTime(state.dates.beforeHour)}` : ''}</span>
          <span>{vp(state)}</span>{place.missing && <small>Not recorded: {place.missing}</small>}
        </div>; }} />
    </div>}
  </div>;
}
