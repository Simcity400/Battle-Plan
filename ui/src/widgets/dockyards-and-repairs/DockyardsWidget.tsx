import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useRef } from 'react';
import type { LiveValues } from '../../charts/live';
import WidgetState from '../../components/WidgetState';
import Table, { CellBar } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import type { TooltipContent } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useShownHour } from '../../dashboard/widgetPerf';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { countText, fixedText, hullShare, percentText, repairsStructure, repairsTexts, shipLabel, shipNameRow, validateRepairs } from '../refits/yardsData';
import { useLivePaint } from '../refits/livePaint';
import { YARD_OPTIONS } from '../refits/yardOptions';
import '../refits/yards.css';
import type { RepairPort } from '../../generated/dashboard';

interface RepairRow { key: string; port: RepairPort; index?: number }

const rowKey = (row: RepairRow) => row.key;
const portName = (port: RepairPort) => port.name ?? 'Port name unavailable';

export default function DockyardsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(YARD_OPTIONS, widget.options);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const root = useRef<HTMLDivElement | null>(null);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'naval-repairs', tag ? { country: countryFromKey(tag) } : null, { active, validate: validateRepairs, structure: repairsStructure });
  const { data, error, retry } = head;
  const shown = useShownHour();
  const painted = useCallback(() => shown(head.shownAt()), [shown, head.shownAt]);
  const texts = useMemo(() => data ? (values: LiveValues<number>) => repairsTexts(data, values) : null, [data]);
  useLivePaint(root, head.live, texts, painted);
  const attach = useCallback((element: HTMLDivElement | null) => { root.current = element; head.root(element); }, [head.root]);
  const shownHour = () => head.hour ?? Math.floor(runtime.clock.getHour());
  const { rows, children } = useMemo(() => {
    const ports = data?.ports ?? [];
    const queues = new Map(ports.map(port => [port.province, port.queue.map((_, index): RepairRow => ({ key: `${port.province}|${index}`, port, index }))]));
    return { rows: ports.map((port): RepairRow => ({ key: String(port.province), port })), children: (row: RepairRow) => row.index === undefined ? queues.get(row.port.province) ?? [] : undefined };
  }, [data]);
  const bars = options.cellBars !== false;
  const columns = useMemo<readonly TableColumn<RepairRow>[]>(() => [
    { key: 'name', header: 'Port or ship', label: 'Port or ship', width: 'minmax(var(--table-name-min), 2fr)', sortable: false,
      value: row => row.index === undefined ? portName(row.port) : shipLabel(row.port.queue[row.index]).name,
      render: row => {
        if (row.index === undefined) return portName(row.port);
        const label = shipLabel(row.port.queue[row.index]);
        return label.source === 'design' ? <span className="yard-unnamed">{label.name}</span> : label.name;
      } },
    { key: 'hull', header: 'Hull', label: 'Hull', numeric: true, width: 'minmax(90px, 1fr)', sortable: false, renderMissing: true,
      value: row => row.index === undefined ? null : hullShare(row.port.queue[row.index]),
      render: row => {
        if (row.index === undefined) return '';
        const share = hullShare(row.port.queue[row.index]);
        return <CellBar value={bars ? share : null} maximum={1}><span data-paint={`hull|${row.port.province}|${row.index}`}>{percentText(share)}</span></CellBar>;
      } },
    { key: 'dockyards', header: 'Dockyards', label: 'Dockyards', numeric: true, width: 'max-content', priority: 2, sortable: false, renderMissing: true,
      value: row => row.index === undefined ? row.port.dockyards : null, render: row => row.index === undefined ? countText(row.port.dockyards) : '' },
    { key: 'priority', header: 'Priority', label: 'Priority', numeric: true, width: 'max-content', priority: 1, sortable: false, renderMissing: true,
      value: row => row.index === undefined ? row.port.priority : null, render: row => row.index === undefined ? countText(row.port.priority) : '' },
  ], [bars]);
  const tooltip = useCallback((row: RepairRow): TooltipContent => repairTooltip(row, dayLabel(shownHour())),
    [head.hour]);
  const menu = useCallback(() => track ? [jumpItem(runtime, track.id, shownHour())] : [],
    [track, runtime, head.hour]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see dockyards and repairs." />;
  if (!tag) return <WidgetState kind="empty" icon="anchor" message="Choose a country in the widget's header." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <div ref={attach} className="yards"><WidgetState kind="loading" label="Loading dockyards…" /></div>;
  return <div ref={attach} className="yards">
    <div className="yards-summary" aria-live="polite">
      <span><b>{data.queuedShips?.toLocaleString('en-US') ?? 'Not recorded'}</b> ships queued for repair</span>
      <span><b>{data.usedDockyards === null ? 'Not recorded' : data.usedDockyards.toLocaleString('en-US')}</b> repair dockyards used{data.repairLimit === null ? '' : <small> of {data.repairLimit.toLocaleString('en-US')}</small>}</span>
    </div>
    {!rows.length ? data.censusComplete ? <WidgetState kind="empty" message="No ports with repairs at this date." />
      : <WidgetState kind="unavailable" message="Repairs were not fully recorded at this date." />
      : <Table rowRevision={row => row.index === undefined ? [portName(row.port), row.port.dockyards, row.port.priority]
        : [shipLabel(row.port.queue[row.index]).name, Math.floor((hullShare(row.port.queue[row.index]) ?? -1) * 100)]}
      key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} label="Repair queues by port" rows={rows} columns={columns}
      rowKey={rowKey} sort={{ column: 'name', direction: 'ascending' }} unsorted tree={{ children }} cellBars={bars}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} tooltip={tooltip} menu={menu} hoursPerSecond={hoursPerSecond} />}
  </div>;
}

function repairTooltip(row: RepairRow, date: string): TooltipContent {
  if (row.index === undefined) return { title: portName(row.port), rows: [
    { label: 'Repair dockyards', value: countText(row.port.dockyards) },
    { label: 'Priority', value: countText(row.port.priority) },
    { label: 'Repair policy', value: row.port.policy ?? 'Not recorded' },
    { label: 'Ships queued', value: String(row.port.queue.length) },
    { label: 'As of', value: date }] };
  const item = row.port.queue[row.index];
  const label = shipLabel(item);
  return { title: label.name, rows: [
    shipNameRow(label.source),
    { label: 'Ship', lookupKey: item?.handle, value: item?.handle },
    { label: 'Hull', value: `${fixedText(item?.hull)} of ${fixedText(item?.maximum)}` },
    { label: 'Place in queue', value: String(row.index + 1) },
    { label: 'Class', value: item?.class ?? 'Not recorded' },
    { label: 'Task force', value: item?.taskForce ?? 'Not recorded' },
    { label: 'Repair policy', value: item?.policy ?? 'Not recorded' },
    { label: 'As of', value: date }] };
}
