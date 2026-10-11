import { useMemo, useRef, useState } from 'react';
import DateTimePicker from '../components/DateTimePicker';
import WidgetState from '../components/WidgetState';
import { useContextMenu } from '../components/ContextMenu';
import DrillParts from '../charts/drill/DrillParts';
import { drawBands, drawPlayhead } from '../charts/draw';
import { jumpItem } from '../charts/jump';
import { lineStyles } from '../charts/palette';
import { runAt } from '../charts/series';
import { buildStack, stackExtent } from '../charts/stack';
import { countryRaceSegments } from '../charts/race/countryRace';
import TimeChart from '../charts/TimeChart';
import type { ChartLayer } from '../charts/TimeChart';
import { useChartTokens } from '../charts/useChartTokens';
import { formatDateTime } from './calendar';
import { contentTitle } from './contentTitles';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { usePlaybackSpeed } from './playback';
import { LEDGER_GAP_NOTE, leafAmount, ledgerCountryAt, ledgerFormat, ledgerRuns, ledgerScale, ledgerShare, ledgerTotals, militaryAirIndex } from './militaryAirLedger';
import type { MilitaryAirLeaf, MilitaryAirReply } from '../generated/dashboard';

const NO_FILTERS = { regions: new Set<number>(), causes: new Set<number>() };

/** The approved parts-and-history drill, sharing the sunburst widget's event scan. */
export default function BombingDrill({ data, runtime, track, tag, identity, from, hour, end, dealt, active, onPick, pickOwner }: {
  data: MilitaryAirReply; runtime: WidgetRuntime; track: BoundTrack; tag: string; identity: string; from: number; hour: number; end: number;
  dealt: boolean; active: boolean; onPick: (hour: number) => void; pickOwner: unknown;
}) {
  const tokens = useChartTokens(), hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const menu = useContextMenu('Bombing damage history actions');
  const index = useMemo(() => militaryAirIndex(data, NO_FILTERS), [data]);
  const row = hour >= from && hour <= end ? ledgerCountryAt(index, tag, from, hour, identity) : null;
  const model = useMemo(() => {
    const aliases = index.series.filter(series => series.country.identity === identity);
    if (!aliases.length) return [];
    const present = aliases.flatMap(series => series.country.present).sort((a, b) => a.firstHour - b.firstHour);
    const country = { ...aliases[0].country, present };
    const groups = new Map<string, MilitaryAirLeaf[]>();
    for (const leaf of aliases[0].leaves) {
      if (leaf.dealt !== dealt) continue;
      const key = leaf.buildingToken ?? String(leaf.buildingId), leaves = groups.get(key) ?? [];
      leaves.push(leaf); groups.set(key, leaves);
    }
    const named = new Map(data.buildings.map(item => [item.id, contentTitle(item.name, item.id, 'Building name unavailable')]));
    const groupsByFinal = [...groups].map(([key, leaves]) => ({ key, leaves, final: leaves.reduce((sum, leaf) => sum + leafAmount(leaf, from, end), 0n) }))
      .sort((a, b) => a.final === b.final ? a.key < b.key ? -1 : a.key > b.key ? 1 : 0 : a.final > b.final ? -1 : 1);
    const styles = lineStyles(groupsByFinal.map(({ key }, preferred) => ({ key, preferred })), tokens.categories);
    return groupsByFinal.map(({ key, leaves }) => ({ key, leaves, label: named.get(key) ?? 'Building type not recorded', ...styles.get(key)!,
      runs: ledgerRuns({ country, leaves, points: ledgerTotals(leaves) }, from, end, dealt, ledgerScale('bombing')) }));
  }, [data, index, identity, dealt, from, end, tokens]);
  const selectionOwner = useMemo(() => ({ pickOwner, client: runtime.client, identity, dealt }), [pickOwner, runtime.client, identity, dealt]);
  const ownerRef = useRef({ owner: selectionOwner, active });
  ownerRef.current = { owner: selectionOwner, active };
  const [selected, setSelected] = useState<{ owner: unknown; id: string } | null>(null);
  const selectedId = selected?.owner === selectionOwner && model.some(part => part.key === selected.id) ? selected.id : null;
  const shown = useMemo(() => selectedId == null ? model : model.filter(part => part.key === selectedId), [model, selectedId]);
  const stack = useMemo(() => buildStack(shown.map(part => part.runs.map(run => ({ ...run, value: Math.max(0, run.value) })))), [shown]);
  const until = Math.min(hour, end);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of countryRaceSegments(stack, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]), hoursPerPixel, true))
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, shown.map(part => part.color));
    },
    extent: range => stackExtent(stack, range[0], Math.min(range[1], until)),
  }], [shown, stack, until]);
  const historyOwner = useMemo(() => ({ selectionOwner, selectedId }), [selectionOwner, selectedId]);
  const total = row ? dealt ? row.dealt : row.taken : null;
  const pickDay = (at: number) => {
    if (!ownerRef.current.active || ownerRef.current.owner !== selectionOwner) return;
    at = Math.floor(at);
    if (at >= from && at <= end) onPick(at);
  };
  const parts = model.map(part => {
    const raw = row ? part.leaves.reduce((sum, leaf) => sum + leafAmount(leaf, from, hour), 0n) : null;
    return { id: part.key, label: part.label, color: part.color, raw, value: raw == null ? null : Number(raw) / ledgerScale('bombing'),
      text: ledgerFormat(raw, 'bombing'), shareText: raw == null || total == null ? undefined : ledgerShare(raw, total) };
  });
  return <div className="military-air-ledger bombing-drill">
    {!row && <>
      <WidgetState kind="unavailable" message="This country or bombing interval was not recorded at this moment." />
      <DateTimePicker label="Choose breakdown day" value={hour} min={from} max={end} otherValue={null} disabled={!active} describedBy="" onChange={pickDay} />
    </>}
    <DrillParts parts={parts} virtual active={active} positiveBars signedShares format={String} showShares={row?.complete ?? false}
      onPick={id => { if (ownerRef.current.active && ownerRef.current.owner === selectionOwner && model.some(part => part.key === id))
        setSelected(before => before?.owner === selectionOwner && before.id === id ? null : { owner: selectionOwner, id }); }}
      totalRow={{ label: dealt ? 'Damage inflicted' : 'Damage suffered', value: ledgerFormat(total, 'bombing') }} />
    <p className="chart-line-key">{selectedId == null ? 'All building types stacked. Select a building type to view its history.' : <>
      History: {model.find(part => part.key === selectedId)?.label}. <button type="button" className="button small"
        onClick={() => { if (ownerRef.current.active && ownerRef.current.owner === selectionOwner) setSelected(null); }}>Show all building types</button>
    </>}</p>
    <TimeChart label="Bombing damage by building type over time" extent={[from, end]} playhead={hour} playback="cursor" afterPlayhead={false}
      hoursPerSecond={hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} branchStart={track.branch.firstHour}
      active={active} layers={layers} revision={stack} pickOwner={historyOwner} overlay={(ctx, frame, chartTokens) => drawPlayhead(ctx, frame, chartTokens, hour)}
      keyHours={[...new Set(shown.flatMap(part => part.runs.map(run => run.firstHour)))].sort((a, b) => a - b)}
      onPick={pickDay} onContextMenu={(at, x, y) => { if (ownerRef.current.active && ownerRef.current.owner === selectionOwner) menu.open(x, y, [jumpItem(runtime, track.id, at)]); }}
      tooltip={at => {
        at = Math.floor(at);
        if (at < from || at > until || !ledgerCountryAt(index, tag, from, at, identity)) return null;
        return { title: formatDateTime(at), rows: shown.map(part => ({ label: part.label, swatch: part.color,
          value: runAt(part.runs, at) ? ledgerFormat(part.leaves.reduce((sum, leaf) => sum + leafAmount(leaf, from, at), 0n), 'bombing') : 'Not recorded' })),
          note: 'Click to pick this moment. ' + LEDGER_GAP_NOTE };
      }} />
    <p className="chart-line-key">Areas show positive amounts; correction rows and totals retain their signs. {row?.complete ? '' : LEDGER_GAP_NOTE}</p>
    {menu.menu}
  </div>;
}
