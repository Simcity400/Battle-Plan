import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { PreparedKey } from '../../charts/PreparedKey';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import type { TooltipContent } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackMotion } from '../../dashboard/playback';
import { pendingReplyAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { censusComplete, costRead, countText, derivedIcText, fixedText, hullShare, nativeIcText, percentText, refitShare, tokenName, yardsAt } from '../../dashboard/navalYards';
import type { YardsAt, YardsReply } from '../../dashboard/navalYards';
import { shipLabel, shipName, shipNameRow } from './shipNames';
import { useYards } from './useYards';
import '../division-log/divisionLog.css';
import './refits.css';

/** A refit line with its ships, or one ship on it. */
interface RefitRow { key: string; kind: 'line' | 'ship'; line: string; ship?: string }

const rowKey = (row: RefitRow) => row.key;

/** The refit line's progress as a share of its cost, for the meter it fills. */
export const lineProgress = (at: YardsAt, line: string): number | null => refitShare(at, line);

/** One country's refit work at the hour shown: each live refit line with its target design,
 * its progress and the ships on it, in the game's order, under one totals line. */
export default function RefitsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const { track, tag, options, data, error, retry } = useYards(widget, runtime, active, 'lines');
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const at = useMemo(() => data && tag ? yardsAt(data, tag, hour) : null, [data, tag, hour]);
  const { rows, children } = useMemo(() => refitRows(at), [at]);
  const readDate = at?.readHour === undefined ? null : dayLabel(at.readHour);
  const columns = useMemo<readonly TableColumn<RefitRow>[]>(() => !data || !at ? [] : [
    { key: 'name', header: 'Refit or ship', label: 'Refit or ship', width: 'minmax(var(--table-name-min), 2fr)', sortable: false,
      value: row => row.kind === 'line' ? lineName(data, at, row.line) : shipName(data, at.groups.get(row.line)?.ships.get(row.ship!)),
      render: row => {
        if (row.kind === 'line') return lineName(data, at, row.line);
        const label = shipLabel(data, at.groups.get(row.line)?.ships.get(row.ship!));
        return label.source === 'design' ? <span className="yard-unnamed">{label.name}</span> : label.name;
      } },
    { key: 'progress', header: 'Progress', label: 'Progress', width: 'minmax(90px, 1fr)', sortable: false, renderMissing: true,
      value: row => row.kind === 'line' ? lineProgress(at, row.line) : null,
      render: row => row.kind === 'line' ? <span className="refits-progress"><ProgressMeter value={lineProgress(at, row.line)} state="live" label="Refit progress" />
        <span>{percentText(lineProgress(at, row.line))}</span></span> : '',
      text: row => row.kind === 'line' ? percentText(lineProgress(at, row.line)) : null },
    { key: 'hull', header: 'Hull', label: 'Hull', numeric: true, width: 'max-content', priority: 2, sortable: false, renderMissing: true,
      value: row => row.kind === 'ship' ? hullShare(at.groups.get(row.line)?.ships.get(row.ship!)) : null,
      render: row => row.kind === 'ship' ? percentText(hullShare(at.groups.get(row.line)?.ships.get(row.ship!))) : '' },
    { key: 'dockyards', header: 'Dockyards', label: 'Dockyards', numeric: true, width: 'max-content', priority: 1, sortable: false, renderMissing: true,
      value: row => row.kind === 'line' ? Number(at.groups.get(row.line)?.measures.get('dockyards') ?? 0n) : null,
      render: row => row.kind === 'line' ? countText(at.groups.get(row.line)?.measures.get('dockyards')) : '' },
  ], [data, at]);
  const tooltip = useCallback((row: RefitRow): TooltipContent => data && at ? refitTooltip(data, at, row, readDate) : { title: '', rows: [] }, [data, at, readDate]);
  const menu = useCallback(() => track && at?.readHour !== undefined ? [jumpItem(runtime, track.id, at.readHour)] : [], [track, at, runtime]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see refits." />;
  if (!tag) return <WidgetState kind="empty" icon="anchor" message="Choose a country in the widget's header." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading refits…" />;
  if (pendingReplyAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!at) return <WidgetState kind="unavailable" message="Refits were not recorded at this time." />;
  const prefix = preparedPrefix(data);
  const total = (key: string) => at.totals.get(key);
  return <div className="refits">
    <PreparedKey pending={prefix?.pending} approximate={prefix?.approximate} />
    <div className="division-log-summary" aria-live="polite">
      <span><b>{total('ships')?.toLocaleString('en-US') ?? 'Not recorded'}</b> ships refitting</span>
      <span><b>{total('lines')?.toLocaleString('en-US') ?? 'Not recorded'}</b> refit lines</span>
      <span><b>{total('dockyards') === undefined ? 'Not recorded' : countText(total('dockyards'))}</b> dockyards</span>
    </div>
    {!rows.length ? censusComplete(at, 'censusComplete') ? <WidgetState kind="empty" message="No refits at this date." />
      : <WidgetState kind="unavailable" message="Refits were not fully recorded at this date." />
      : <Table key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} label="Refit lines" rows={rows} columns={columns}
        rowKey={rowKey} sort={{ column: 'name', direction: 'ascending' }} unsorted tree={{ children }}
        density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} tooltip={tooltip} menu={menu}
        hoursPerSecond={hoursPerSecond} parity={{ complete: true, item: row => row }} />}
  </div>;
}


function lineName(data: YardsReply, at: YardsAt, line: string): string {
  return tokenName(data, at.groups.get(line)?.measures.get('targetName'), data.groups.find(group => group.id === line)?.name ?? 'Design name unavailable');
}

/** Live lines in their recorded order, each with the ships whose membership the hour dates. */
function refitRows(at: YardsAt | null): { rows: RefitRow[]; children: (row: RefitRow) => RefitRow[] | undefined } {
  if (!at) return { rows: [], children: () => undefined };
  const lines = [...at.groups.values()].filter(group => group.measures.get('live') === 1n);
  const ships = new Map(lines.map(line => [line.id, [...line.ships.values()].filter(ship => ship.measures.get('present') === 1n)
    .map(ship => ({ key: `${line.id}|${ship.id}`, kind: 'ship' as const, line: line.id, ship: ship.id }))]));
  return { rows: lines.map(line => ({ key: line.id, kind: 'line', line: line.id })), children: row => row.kind === 'line' ? ships.get(row.line) ?? [] : undefined };
}

function refitTooltip(data: YardsReply, at: YardsAt, row: RefitRow, readDate: string | null): TooltipContent {
  const group = at.groups.get(row.line);
  if (row.kind === 'line') return { title: lineName(data, at, row.line), rows: [
    { label: 'Progress', value: lineProgress(at, row.line) === null ? 'Not recorded' : percentText(lineProgress(at, row.line)) },
    { label: 'Work done', value: nativeIcText(group?.measures.get('progress')) },
    { label: 'Cost', value: nativeIcText(group?.measures.get('cost')) },
    ...costRead(at) === undefined ? [] : [{ label: 'Cost as of', value: dayLabel(costRead(at)!) }],
    { label: 'Remaining cost', value: derivedIcText(group?.measures.get('remainingCostRaw')) },
    { label: 'Dockyards', value: countText(group?.measures.get('dockyards')) },
    ...readDate ? [{ label: 'As of', value: readDate }] : [] ] };
  const ship = group?.ships.get(row.ship!);
  const label = shipLabel(data, ship);
  return { title: label.name, rows: [
    shipNameRow(label.source),
    { label: 'Ship', lookupKey: row.ship, value: row.ship },
    { label: 'Hull', value: `${fixedText(ship?.measures.get('hull'))} of ${fixedText(ship?.measures.get('maximum'))}` },
    { label: 'Design', value: tokenName(data, ship?.measures.get('designName'), 'Design name unavailable') },
    ...readDate ? [{ label: 'As of', value: readDate }] : [] ] };
}
