import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import Table, { CellBar } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { PreparedKey } from '../../charts/PreparedKey';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import type { TooltipContent } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackMotion } from '../../dashboard/playback';
import { pendingReplyAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { censusComplete, countText, fixedText, hullShare, percentText, tokenName, yardsAt } from '../../dashboard/navalYards';
import type { YardsAt, YardsReply } from '../../dashboard/navalYards';
import { shipLabel, shipName, shipNameRow } from '../refits/shipNames';
import { useYards } from '../refits/useYards';
import '../division-log/divisionLog.css';
import './dockyards.css';

/** A port with its repair queue, or one queued ship. */
interface RepairRow { key: string; kind: 'port' | 'ship'; port: string; ship?: string; ordinal?: number }

const rowKey = (row: RepairRow) => row.key;

/** One country's ports and their repair queues at the hour shown: each port's dockyards and
 * priority, and the ships queued there in the game's order with their hull. */
export default function DockyardsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const shown = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = shown && resolveWindow(widget, shown, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? shown?.branch.lastHour ?? 0);
  const { track, tag, options, data, error, retry } = useYards(widget, runtime, active, hour);
  const at = useMemo(() => data && tag ? yardsAt(data) : null, [data, tag]);
  const { rows, children } = useMemo(() => repairRows(at), [at]);
  const readDate = at?.readHour === undefined ? null : dayLabel(at.readHour);
  const columns = useMemo<readonly TableColumn<RepairRow>[]>(() => !data || !at ? [] : [
    { key: 'name', header: 'Port or ship', label: 'Port or ship', width: 'minmax(var(--table-name-min), 2fr)', sortable: false,
      value: row => row.kind === 'port' ? portName(data, at, row.port) : shipName(data, at.groups.get(row.port)?.ships.get(row.ship!)),
      render: row => {
        if (row.kind === 'port') return portName(data, at, row.port);
        const label = shipLabel(data, at.groups.get(row.port)?.ships.get(row.ship!));
        return label.source === 'design' ? <span className="yard-unnamed">{label.name}</span> : label.name;
      } },
    { key: 'hull', header: 'Hull', label: 'Hull', numeric: true, width: 'minmax(90px, 1fr)', sortable: false, renderMissing: true,
      value: row => row.kind === 'ship' ? hullShare(at.groups.get(row.port)?.ships.get(row.ship!)) : null,
      render: row => {
        if (row.kind === 'port') return '';
        const share = hullShare(at.groups.get(row.port)?.ships.get(row.ship!));
        return <CellBar value={share} maximum={1}>{percentText(share)}</CellBar>;
      }, text: row => row.kind === 'ship' ? percentText(hullShare(at.groups.get(row.port)?.ships.get(row.ship!))) : null },
    { key: 'dockyards', header: 'Dockyards', label: 'Dockyards', numeric: true, width: 'max-content', priority: 2, sortable: false, renderMissing: true,
      value: row => row.kind === 'port' ? numberOf(at.groups.get(row.port)?.measures.get('dockyards')) : null,
      render: row => row.kind === 'port' ? countText(at.groups.get(row.port)?.measures.get('dockyards')) : '' },
    { key: 'priority', header: 'Priority', label: 'Priority', numeric: true, width: 'max-content', priority: 1, sortable: false, renderMissing: true,
      value: row => row.kind === 'port' ? numberOf(at.groups.get(row.port)?.measures.get('priority')) : null,
      render: row => row.kind === 'port' ? (at.groups.get(row.port)?.measures.get('priority')?.toString() ?? '—') : '' },
  ], [data, at]);
  const tooltip = useCallback((row: RepairRow): TooltipContent => data && at ? repairTooltip(data, at, row, readDate) : { title: '', rows: [] }, [data, at, readDate]);
  const menu = useCallback(() => track && at?.readHour !== undefined ? [jumpItem(runtime, track.id, at.readHour)] : [], [track, at, runtime]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see dockyards and repairs." />;
  if (!tag) return <WidgetState kind="empty" icon="anchor" message="Choose a country in the widget's header." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading dockyards…" />;
  if (pendingReplyAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!at) return <WidgetState kind="unavailable" message="Repairs were not recorded at this time." />;
  const prefix = preparedPrefix(data);
  const used = at.totals.get('usedDockyards'), limit = at.totals.get('repairLimit'), queued = at.totals.get('distinctQueuedShips');
  return <div className="dockyards">
    <PreparedKey pending={prefix?.pending} approximate={prefix?.approximate} />
    <div className="division-log-summary" aria-live="polite">
      <span><b>{queued?.toLocaleString('en-US') ?? 'Not recorded'}</b> ships queued for repair</span>
      <span><b>{used === undefined ? 'Not recorded' : used.toLocaleString('en-US')}</b> repair dockyards used{limit === undefined ? '' : <small> of {limit.toLocaleString('en-US')}</small>}</span>
    </div>
    {!rows.length ? censusComplete(at, 'queueCensusComplete') ? <WidgetState kind="empty" message="No ports with repairs at this date." />
      : <WidgetState kind="unavailable" message="Repairs were not fully recorded at this date." />
      : <Table key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} label="Repair queues by port" rows={rows} columns={columns}
        rowKey={rowKey} sort={{ column: 'name', direction: 'ascending' }} unsorted tree={{ children }} cellBars={options.cellBars !== false}
        density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} tooltip={tooltip} menu={menu}
        hoursPerSecond={hoursPerSecond} parity={{ complete: true, item: row => row }} />}
  </div>;
}

const numberOf = (value: bigint | undefined) => value === undefined ? null : Number(value);

function portName(data: YardsReply, at: YardsAt, port: string): string {
  return tokenName(data, at.groups.get(port)?.measures.get('name'), 'Port name unavailable');
}

/** Ports in recorded order (largest priority first, then province), each with its queue. */
function repairRows(at: YardsAt | null): { rows: RepairRow[]; children: (row: RepairRow) => RepairRow[] | undefined } {
  if (!at) return { rows: [], children: () => undefined };
  const ports = [...at.groups.values()].filter(group => group.measures.get('present') === 1n)
    .sort((a, b) => Number((b.measures.get('priority') ?? 0n) - (a.measures.get('priority') ?? 0n)) || Number(a.id) - Number(b.id));
  const queues = new Map(ports.map(port => [port.id, port.queue.map(entry => ({ key: `${port.id}|${entry.ordinal}`, kind: 'ship' as const, port: port.id, ship: entry.ship, ordinal: entry.ordinal }))]));
  return { rows: ports.map(port => ({ key: port.id, kind: 'port', port: port.id })), children: row => row.kind === 'port' ? queues.get(row.port) ?? [] : undefined };
}

function repairTooltip(data: YardsReply, at: YardsAt, row: RepairRow, readDate: string | null): TooltipContent {
  const group = at.groups.get(row.port);
  if (row.kind === 'port') return { title: portName(data, at, row.port), rows: [
    { label: 'Repair dockyards', value: countText(group?.measures.get('dockyards')) },
    { label: 'Priority', value: group?.measures.get('priority')?.toString() ?? '—' },
    { label: 'Repair policy', value: tokenName(data, group?.measures.get('policy'), 'Not recorded') },
    { label: 'Ships queued', value: group?.measures.get('queueCount')?.toString() ?? '—' },
    ...readDate ? [{ label: 'As of', value: readDate }] : [] ] };
  const ship = group?.ships.get(row.ship!);
  const label = shipLabel(data, ship);
  return { title: label.name, rows: [
    shipNameRow(label.source),
    { label: 'Ship', lookupKey: row.ship, value: row.ship },
    { label: 'Hull', value: `${fixedText(ship?.measures.get('hull'))} of ${fixedText(ship?.measures.get('maximum'))}` },
    { label: 'Place in queue', value: String((row.ordinal ?? 0) + 1) },
    { label: 'Task force', value: tokenName(data, ship?.measures.get('taskName'), 'Not recorded') },
    { label: 'Repair policy', value: tokenName(data, ship?.measures.get('policy'), 'Not recorded') },
    ...readDate ? [{ label: 'As of', value: readDate }] : [] ] };
}
