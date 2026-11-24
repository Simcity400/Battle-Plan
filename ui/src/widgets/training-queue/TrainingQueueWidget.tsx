import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import Table from '../../charts/table/Table';
import type { TableColumn, TableTree } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import { usNumber } from '../../charts/numberFormat';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime.ts';
import GateState from '../../dashboard/GateState';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS } from './definition';
import { divisionName, divisionTooltip, orderIcon, equipmentFill, lowestFill, orderName, orderTooltip, percent, queueRows, queueStructure, trainingRatio, validateTrainingQueue } from './trainingQueueData';
import type { QueueRow } from './trainingQueueData';
import './trainingQueue.css';

const rowKey = (row: QueueRow) => row.key;
const NO_SORT = { column: 'name', direction: 'ascending' } as const;

function meterCell({ ratio, label }: { ratio: number | null; label: string }) {
  return ratio === null ? <span className="training-queue-missing" data-tooltip="Not recorded">—</span>
    : <span className="training-queue-meter"><ProgressMeter value={ratio} state="live" label={label} /><span>{percent(ratio)}</span></span>;
}

export default function TrainingQueueWidget({ widget, runtime, active, setOption, onDiagnostics, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, SPEC, active, { country: { stored: widget.options?.country, setOption } });
  const country = gate.country!;
  const hour = gate.hour;
  const tag = gate.open ? country.tag : null;
  const countryLabel = useCountryNameAt(runtime.client, track, tag ?? country.chosen, hour, active);
  const countryName = countryLabel.name;
  const nameDetail = countryLabel.nameFailure ? <span data-detail={countryLabel.nameFailure.message}>{countryName}</span> : undefined;
  const options = resolveOptions(OPTIONS, widget.options);
  const state = useHeadProduct(runtime, gate, 'training-queue', tag ? { country: countryFromKey(tag) } : null,
    { active, validate: validateTrainingQueue, structure: queueStructure });
  const data = state.data;
  const rows = useMemo(() => data ? queueRows(data) : [], [data]);
  const [closed, setClosed] = useState<ReadonlySet<string>>(new Set());
  const tree = useMemo<TableTree<QueueRow>>(() => ({
    children: row => row.kind === 'order' ? row.children : undefined,
    open: row => !closed.has(row.key),
    onToggle: (row, open) => setClosed(before => { const next = new Set(before); if (open) next.delete(row.key); else next.add(row.key); return next; }),
  }), [closed]);
  const viewportHeight = useViewportHeight();
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  const columns = useMemo<readonly TableColumn<QueueRow>[]>(() => [
    { key: 'name', header: 'Deployment', label: 'Deployment', width: 'minmax(var(--table-name-min), 2fr)', sortable: false, value: () => null, renderMissing: true,
      text: row => row.kind === 'order' ? `${orderName(row.order)} ×${row.order.amount}` : divisionName(row.division),
      render: row => row.kind === 'order'
        ? <span className="training-queue-name" data-tooltip={row.order.template}>{orderIcon(row.order) ? <img src={orderIcon(row.order)!} alt="" /> : <ArtFallback icon="helmet" name={orderName(row.order)} reason={row.order.iconMissing} />}
          <span>{orderName(row.order)}</span><small>×{usNumber.format(row.order.amount)}</small></span>
        : <span className="training-queue-name"><span>{divisionName(row.division)}</span></span> },
    { key: 'training', header: 'Training', label: 'Training', width: 'minmax(calc(var(--space-6) * 3), 1fr)', sortable: false, value: () => null, renderMissing: true,
      text: row => row.kind === 'order' ? `${row.order.divisions.length} in training` : (r => r === null ? 'Not recorded' : percent(r))(trainingRatio(row.division)),
      render: row => row.kind === 'order'
        ? closed.has(row.key) || !row.order.divisions.length ? <span className="training-queue-missing">{row.order.divisions.length ? `${row.order.divisions.length} in training` : 'No divisions in training'}</span> : ''
        : meterCell({ ratio: trainingRatio(row.division), label: 'Training' }) },
    { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(calc(var(--space-6) * 3), 1fr)', sortable: false, priority: 1, value: () => null, renderMissing: true,
      text: row => { const fill = row.kind === 'order' ? lowestFill(row.order) : equipmentFill(row.division); return fill === null ? 'Not recorded' : percent(fill); },
      render: row => row.kind === 'order' ? closed.has(row.key) ? meterCell({ ratio: lowestFill(row.order), label: 'Lowest equipment fill' }) : ''
        : meterCell({ ratio: equipmentFill(row.division), label: 'Equipment' }) },
    { key: 'manpower', header: 'Manpower', label: 'Manpower', hint: 'Available / needed', numeric: true, width: 'max-content', sortable: false, priority: 2, value: () => null, renderMissing: true,
      text: row => row.kind === 'order' ? '' : `${usNumber.format(row.division.manpower)} / ${usNumber.format(row.division.manpowerNeeded)}`,
      render: row => row.kind === 'order' ? '' : `${usNumber.format(row.division.manpower)} / ${usNumber.format(row.division.manpowerNeeded)}` },
    { key: 'location', header: 'Location', label: 'Location', width: 'minmax(calc(var(--space-6) * 3), 1fr)', sortable: false, priority: 3, value: () => null, renderMissing: true,
      text: row => row.kind === 'order' ? row.order.locationName ?? '—' : '',
      render: row => row.kind === 'order' ? row.order.locationName === null
        ? <span className="training-queue-missing" data-tooltip={row.order.locationMissing ?? 'Not recorded'}>—</span>
        : <span title={row.order.locationName} data-tooltip={row.order.location ? `Province ${row.order.location}` : undefined}>{row.order.locationName}</span> : '' },
    { key: 'priority', header: 'Priority', label: 'Priority', width: 'minmax(calc(var(--space-6) * 2), 1fr)', sortable: false, priority: 4, value: () => null, renderMissing: true,
      text: row => row.kind === 'order' ? row.order.priorityName ?? '—' : '',
      render: row => row.kind === 'order' ? row.order.priorityName === null
        ? <span className="training-queue-missing" data-tooltip={row.order.priorityMissing ?? 'Not recorded'}>—</span>
        : <span title={row.order.priorityName}>{row.order.priorityName}</span> : '' },
  ], [closed]);
  const tooltip = useCallback((row: QueueRow) => { const at = data?.readHour ?? hour ?? Number.NaN;
    return fitTooltip([row.kind === 'order' ? orderTooltip(row.order, at) : divisionTooltip(row.division, at)], viewportHeight); }, [data, hour, viewportHeight]);
  const training = data ? data.orders.reduce((sum, order) => sum + order.divisions.length, 0) : 0;
  const pill = headerSlot && data && data.missing === null && createPortal(<span className="pill">{training} in training</span>, headerSlot);
  let body;
  if (!track) body = <WidgetState kind="empty" message="Load a campaign to see the training queue." />;
  else if (!gate.open) body = <GateState gate={gate} loading="Loading the training queue…" choose="Choose a country to see its training queue."
    absent={nameDetail ? 'This country does not exist at this date.' : `${countryName} does not exist at this date.`} detail={nameDetail} />;
  else if (state.error) body = <WidgetState kind="error" failure={state.error} onRetry={state.retry} />;
  else if (!data) body = <WidgetState kind="loading" label="Loading the training queue…" />;
  else if (data.missing !== null) body = <WidgetState kind="unavailable" message="The training queue wasn’t recorded at this time." detail={nameDetail} />;
  else if (!rows.length) body = <WidgetState kind="empty" message={nameDetail ? 'No divisions in training.' : `${countryName} is not training any divisions.`} detail={nameDetail} />;
  else body = <Table rowRevision={row => [row, columns]} label="Training queue" rows={rows} columns={columns} rowKey={rowKey} tree={tree} unsorted sort={NO_SORT}
    cellBars={options.cellBars === true} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
    tooltip={tooltip} menu={() => data.readHour === null ? [] : [jumpItem(runtime, track.id, data.readHour)]} />;
  return <div className="training-queue" ref={state.root} data-detail={countryLabel.nameFailure?.message}>{pill}{country.chosen && country.error && country.historyHeld ? <WidgetState kind="error" failure={country.error} onRetry={country.retry} /> : null}{body}</div>;
}
