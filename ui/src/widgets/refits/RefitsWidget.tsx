import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useRef } from 'react';
import type { LiveValues } from '../../charts/live';
import WidgetState from '../../components/WidgetState';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import ProgressMeter from '../../charts/meter/ProgressMeter';
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
import { countText, fixedText, hullShare, icText, percentText, refitShare, refitsMissing, refitsStructure, refitsTexts, shipLabel, shipNameRow, validateRefits } from './yardsData';
import { useLivePaint } from './livePaint';
import { YARD_OPTIONS } from './yardOptions';
import './yards.css';
import type { RefitLine } from '../../generated/dashboard';
import type { RefitsReply } from './yardsData';

interface RefitRow { key: string; line: RefitLine; ship?: number }

const rowKey = (row: RefitRow) => row.key;
const lineName = (line: RefitLine) => line.target ?? 'Design name unavailable';

export default function RefitsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(YARD_OPTIONS, widget.options);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const root = useRef<HTMLDivElement | null>(null);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'naval-refits', tag ? { country: countryFromKey(tag) } : null, { active, validate: validateRefits, structure: refitsStructure });
  const { data, error, retry } = head;
  const shown = useShownHour();
  const painted = useCallback(() => shown(head.shownAt()), [shown, head.shownAt]);
  const texts = useMemo(() => data ? (values: LiveValues<number>) => refitsTexts(data, values) : null, [data]);
  useLivePaint(root, head.live, texts, painted);
  const attach = useCallback((element: HTMLDivElement | null) => { root.current = element; head.root(element); }, [head.root]);
  const shownHour = () => head.hour ?? Math.floor(runtime.clock.getHour());
  const { rows, children } = useMemo(() => {
    const lines = data?.lines ?? [];
    const ships = new Map(lines.map(line => [line.handle, line.ships.map((item, index): RefitRow => ({ key: `${line.handle}|${item.handle}`, line, ship: index }))]));
    return { rows: lines.map((line): RefitRow => ({ key: line.handle, line })), children: (row: RefitRow) => row.ship === undefined ? ships.get(row.line.handle) ?? [] : undefined };
  }, [data]);
  const columns = useMemo<readonly TableColumn<RefitRow>[]>(() => !data ? [] : [
    { key: 'name', header: 'Refit or ship', label: 'Refit or ship', width: 'minmax(var(--table-name-min), 2fr)', sortable: false,
      value: row => row.ship === undefined ? lineName(row.line) : shipLabel(row.line.ships[row.ship]).name,
      render: row => {
        if (row.ship === undefined) return lineName(row.line);
        const label = shipLabel(row.line.ships[row.ship]);
        return label.source === 'design' ? <span className="yard-unnamed">{label.name}</span> : label.name;
      } },
    { key: 'progress', header: 'Progress', label: 'Progress', width: 'minmax(90px, 1fr)', sortable: false, renderMissing: true,
      value: row => row.ship === undefined ? refitShare(data, row.line) : null,
      render: row => row.ship === undefined ? <span className="refits-progress"><ProgressMeter value={refitShare(data, row.line)} state="live" label="Refit progress" />
        <span data-paint={`progress|${row.line.handle}`}>{percentText(refitShare(data, row.line))}</span></span> : '' },
    { key: 'hull', header: 'Hull', label: 'Hull', numeric: true, width: 'max-content', priority: 2, sortable: false, renderMissing: true,
      value: row => row.ship === undefined ? null : hullShare(row.line.ships[row.ship]),
      render: row => row.ship === undefined ? '' : <span data-paint={`hull|${row.line.handle}|${row.line.ships[row.ship].handle}`}>{percentText(hullShare(row.line.ships[row.ship]))}</span> },
    { key: 'dockyards', header: 'Dockyards', label: 'Dockyards', numeric: true, width: 'max-content', priority: 1, sortable: false, renderMissing: true,
      value: row => row.ship === undefined ? row.line.dockyards : null, render: row => row.ship === undefined ? countText(row.line.dockyards) : '' },
  ], [data]);
  const tooltip = useCallback((row: RefitRow): TooltipContent => data ? refitTooltip(data, row, dayLabel(shownHour())) : { title: '', rows: [] },
    [data, head.hour]);
  const menu = useCallback(() => track ? [jumpItem(runtime, track.id, shownHour())] : [],
    [track, runtime, head.hour]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  if (!track) return <WidgetState kind="empty" icon="anchor" message="Load a campaign to see refits." />;
  if (!tag) return <WidgetState kind="empty" icon="anchor" message="Choose a country in the widget's header." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <div ref={attach} className="yards"><WidgetState kind="loading" label="Loading refits…" /></div>;
  const missing = refitsMissing(data);
  if (missing) return <div ref={attach} className="yards"><WidgetState kind="unavailable" message={missing.message} reason={missing.reason} /></div>;
  return <div ref={attach} className="yards">
    <div className="yards-summary" aria-live="polite">
      <span><b>{data.ships?.toLocaleString('en-US') ?? 'Not recorded'}</b> ships refitting</span>
      <span><b>{data.censusComplete ? data.lines.length.toLocaleString('en-US') : 'Not recorded'}</b> refit lines</span>
      <span><b>{countText(data.dockyards)}</b> dockyards</span>
    </div>
    {!rows.length ? data.censusComplete ? <WidgetState kind="empty" message="No refits at this date." />
      : <WidgetState kind="unavailable" message="Refits were not fully recorded at this date." />
      : <Table rowRevision={row => row.ship === undefined ? [lineName(row.line), Math.floor((refitShare(data, row.line) ?? -1) * 100), row.line.dockyards]
        : [shipLabel(row.line.ships[row.ship]).name, Math.floor((hullShare(row.line.ships[row.ship]) ?? -1) * 100)]}
      key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} label="Refit lines" rows={rows} columns={columns}
      rowKey={rowKey} sort={{ column: 'name', direction: 'ascending' }} unsorted tree={{ children }}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} tooltip={tooltip} menu={menu} hoursPerSecond={hoursPerSecond} />}
  </div>;
}

function refitTooltip(reply: RefitsReply, row: RefitRow, date: string): TooltipContent {
  if (row.ship === undefined) {
    const share = refitShare(reply, row.line);
    return { title: lineName(row.line), rows: [
      { label: 'Progress', value: share === null ? 'Not recorded' : percentText(share) },
      { label: 'Work done', value: icText(row.line.progress) },
      { label: 'Cost', value: icText(row.line.cost) },
      ...reply.costRead === null ? [] : [{ label: 'Cost as of', value: dayLabel(reply.costRead) }],
      { label: 'Remaining cost', value: icText(row.line.remaining) },
      { label: 'Dockyards', value: countText(row.line.dockyards) },
      { label: 'As of', value: date }] };
  }
  const item = row.line.ships[row.ship];
  const label = shipLabel(item);
  return { title: label.name, rows: [
    shipNameRow(label.source),
    { label: 'Ship', lookupKey: item?.handle, value: item?.handle },
    { label: 'Hull', value: `${fixedText(item?.hull)} of ${fixedText(item?.maximum)}` },
    { label: 'Design', value: item?.design ?? 'Design name unavailable' },
    { label: 'Class', value: item?.class ?? 'Not recorded' },
    { label: 'Task force', value: item?.taskForce ?? 'Not recorded' },
    { label: 'Repair policy', value: item?.policy ?? 'Not recorded' },
    { label: 'As of', value: date }] };
}
