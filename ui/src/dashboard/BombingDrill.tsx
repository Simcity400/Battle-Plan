import { useMemo, useRef, useState } from 'react';
import DateTimePicker from '../components/DateTimePicker';
import WidgetState from '../components/WidgetState';
import { useContextMenu } from '../components/ContextMenu';
import DrillParts from '../charts/drill/DrillParts';
import type { LiveDrillParts } from '../charts/drill/DrillParts';
import { liveAt, mapLive } from '../charts/live';
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
import { usePlaybackMotion } from './playback';
import { BOMBING_GAP_NOTE, ledgerCountryAt, ledgerFormat, ledgerLabel, ledgerRuns, ledgerScale, ledgerShare, ledgerTotals, militaryAirIndex, partAmount } from './militaryAirLedger';
import type { MilitaryAirPart } from '../generated/dashboard';
import type { MilitaryAirLedger } from './militaryAirLedger';

export default function BombingDrill({ data, runtime, track, tag, identity, from, hour, end, dealt, active, onPick, pickOwner, headAt }: {
  data: MilitaryAirLedger; runtime: WidgetRuntime; track: BoundTrack; tag: string; identity: string; from: number; hour: number; end: number;
  dealt: boolean; active: boolean; onPick: (hour: number) => void; pickOwner: unknown;
  headAt?: ((position: number) => number | null) | null;
}) {
  const tokens = useChartTokens(), hoursPerSecond = usePlaybackMotion(runtime.clock);
  const menu = useContextMenu('Bombing damage history actions');
  const index = useMemo(() => militaryAirIndex(data), [data]);
  const row = hour >= from && hour <= end ? ledgerCountryAt(index, tag, from, hour, identity) : null;
  const model = useMemo(() => {
    const aliases = index.series.filter(series => series.country.identity === identity);
    if (!aliases.length) return [];
    const present = aliases.flatMap(series => series.country.present).sort((a, b) => a.firstHour - b.firstHour);
    const country = { ...aliases[0].country, present };
    const groups = new Map<string, MilitaryAirPart[]>();
    for (const part of aliases[0].parts) {
      if (part.dealt !== dealt || part.buildingId === null) continue;
      const key = part.buildingToken ?? String(part.buildingId), parts = groups.get(key) ?? [];
      parts.push(part); groups.set(key, parts);
    }
    const named = new Map(data.buildings.map(item => [item.id, contentTitle(item.name, item.id, 'Building name unavailable')]));
    const groupsByFinal = [...groups].map(([key, parts]) => ({ key, parts, final: parts.reduce((sum, part) => sum + partAmount(part, from, end), 0n) }))
      .sort((a, b) => a.final === b.final ? a.key < b.key ? -1 : a.key > b.key ? 1 : 0 : a.final > b.final ? -1 : 1);
    const styles = lineStyles(groupsByFinal.map(({ key }, preferred) => ({ key, preferred })), tokens.categories);
    return groupsByFinal.map(({ key, parts }) => ({ key, parts, label: named.get(key) ?? 'Building type not recorded', ...styles.get(key)!,
      runs: ledgerRuns({ country, parts, points: ledgerTotals(parts), openings: [] }, from, end, dealt, ledgerScale('bombing')) }));
  }, [data, index, identity, dealt, from, end, tokens]);
  const selectionOwner = useMemo(() => ({ pickOwner, client: runtime.client, identity, dealt }), [pickOwner, runtime.client, identity, dealt]);
  const ownerRef = useRef({ owner: selectionOwner, active });
  ownerRef.current = { owner: selectionOwner, active };
  const [selected, setSelected] = useState<{ owner: unknown; id: string } | null>(null);
  const selectedId = selected?.owner === selectionOwner && model.some(part => part.key === selected.id) ? selected.id : null;
  const shown = useMemo(() => selectedId == null ? model : model.filter(part => part.key === selectedId), [model, selectedId]);
  const stack = useMemo(() => buildStack(shown.map(part => part.runs.map(run => ({ ...run, value: Math.max(0, run.value) })))), [shown]);
  const live = useMemo(() => headAt ? liveAt(runtime.clock, position => headAt(position) ?? hour, at => at) : null,
    [headAt, runtime.clock]);
  const reach = useRef(() => hour);
  reach.current = () => Math.min(live?.hour() ?? hour, end);
  const until = Math.min(hour, end);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of countryRaceSegments(stack, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]), hoursPerPixel, true))
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, shown.map(part => part.color));
    },
    extent: range => stackExtent(stack, range[0], Math.min(range[1], reach.current())),
  }], [shown, stack, until]);
  const historyOwner = useMemo(() => ({ selectionOwner, selectedId }), [selectionOwner, selectedId]);
  const partsAt = (at: number) => {
    const held = at >= from && at <= end ? ledgerCountryAt(index, tag, from, at, identity) : null;
    const sum = held ? dealt ? held.dealt : held.taken : null;
    return { total: sum, parts: model.map(part => {
      const raw = held ? part.parts.reduce((amount, item) => amount + partAmount(item, from, at), 0n) : null;
      return { id: part.key, label: part.label, color: part.color, raw, value: raw == null ? null : Number(raw) / ledgerScale('bombing'),
        text: ledgerLabel(raw, 'bombing'), shareText: raw == null || sum == null ? undefined : ledgerShare(raw, sum) };
    }) };
  };
  const { parts, total } = partsAt(hour);
  const build = useRef(partsAt);
  build.current = partsAt;
  const liveParts = useMemo<LiveDrillParts | undefined>(() => live ? mapLive(live, at => {
    const made = build.current(at);
    return { parts: made.parts, totalText: ledgerLabel(made.total, 'bombing') };
  }) : undefined, [live, model, index, tag, identity, from, end, dealt]);
  const pickDay = (at: number) => {
    if (!ownerRef.current.active || ownerRef.current.owner !== selectionOwner) return;
    at = Math.floor(at);
    if (at >= from && at <= end) onPick(at);
  };
  return <div className="military-air-ledger bombing-drill">
    {!row && <>
      <WidgetState kind="unavailable" message="This country or bombing interval was not recorded at this moment." />
      <DateTimePicker label="Choose breakdown day" value={hour} min={from} max={end} otherValue={null} disabled={!active} describedBy="" onChange={pickDay} />
    </>}
    <DrillParts parts={parts} live={liveParts} virtual active={active} positiveBars signedShares format={String} showShares={row?.complete ?? false}
      onPick={id => { if (ownerRef.current.active && ownerRef.current.owner === selectionOwner && model.some(part => part.key === id))
        setSelected(before => before?.owner === selectionOwner && before.id === id ? null : { owner: selectionOwner, id }); }}
      totalRow={{ label: dealt ? 'Damage inflicted' : 'Damage suffered', value: ledgerLabel(total, 'bombing') }} />
    <p className="chart-line-key">{selectedId == null ? 'All building types' : <>
      History: {model.find(part => part.key === selectedId)?.label}. <button type="button" className="button small"
        onClick={() => { if (ownerRef.current.active && ownerRef.current.owner === selectionOwner) setSelected(null); }}>Show all building types</button>
    </>}</p>
    <TimeChart label="Bombing damage by building type over time" extent={[from, end]} playhead={hour} liveHead={live ?? undefined} playback="cursor" afterPlayhead={false}
      hoursPerSecond={hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} branchStart={track.branch.firstHour}
      active={active} layers={layers} revision={stack} pickOwner={historyOwner} overlay={live ? undefined : (ctx, frame, chartTokens) => drawPlayhead(ctx, frame, chartTokens, hour)}
      keyHours={[...new Set(shown.flatMap(part => part.runs.map(run => run.firstHour)))].sort((a, b) => a - b)}
      onPick={pickDay} onContextMenu={(at, x, y) => { if (ownerRef.current.active && ownerRef.current.owner === selectionOwner) menu.open(x, y, [jumpItem(runtime, track.id, at)]); }}
      tooltip={at => {
        at = Math.floor(at);
        const current = ledgerCountryAt(index, tag, from, at, identity);
        if (at < from || at > reach.current() || !current) return null;
        return { title: formatDateTime(at), rows: shown.map(part => ({ label: part.label, swatch: part.color,
          value: runAt(part.runs, at) ? ledgerFormat(part.parts.reduce((sum, item) => sum + partAmount(item, from, at), 0n), 'bombing') : 'Not recorded' })),
          note: current.complete ? undefined : BOMBING_GAP_NOTE };
      }} />
    <p className="chart-line-key">Areas show positive amounts; correction rows and totals retain their signs. {row?.complete ? '' : BOMBING_GAP_NOTE}</p>
    {menu.menu}
  </div>;
}
