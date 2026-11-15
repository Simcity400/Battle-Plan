import { createPortal } from 'react-dom';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useAtHead } from '../../dashboard/atHead';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { nameReadFailure, nameReadText } from '../../dashboard/nameReads.ts';
import { failureDetails } from '../../dashboard/queryError.ts';
import { useShownEachFrame } from '../../dashboard/widgetPerf.ts';
import { selectedTags } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import type { WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { DEFAULT_SORT, OPTIONS, SHEET } from './definition';
import { designRevision, designTypes, designsAt, designsKnown, designsSince, seenNote, seenText, validateDesigns } from './designsData';
import type { DesignRow } from './designsData';
import './equipmentDesigns.css';
import type { EquipmentDesignsReply } from '../../generated/dashboard';

const NO_ROWS: readonly DesignRow[] = [];
const NO_WARNINGS: readonly string[] = [];
const OBSOLETE_SORT = { column: 'seenObsolete', direction: 'descending' } as const;
const UNNAMED = 'Design name unavailable';

/** The country's designs over the branch, read once; the rows at an hour are worked out here. */
function useDesigns(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-designs', params: { branchId: track.branch.id, tag } } : null;
  return { track, tag, ...useProduct<EquipmentDesignsReply>(runtime.client, request, active, validateDesigns, track?.source) };
}

/** The Included types setting lists the equipment types the country's designs use. */
export function useDesignChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useDesigns(widget, runtime, true);
  return { included: data ? designTypes(data).map(type => ({ value: setToken(type.value), label: type.label })) : [] };
}

/** One country's saved designs at the date shown: current ones, then the obsolete ones in a closed disclosure, each
 * with when it and its obsolete flag were first read. Following playback it renders only where a design's row changes. */
export default function EquipmentDesignsWidget({ widget, runtime, active, headerSlot, onDiagnostics, setOption }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const { track, tag, data, error, retry } = useDesigns(widget, runtime, active);
  const shown = useAtHead(runtime.clock, active && widget.time.mode === 'follow', clockHour => {
    const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
    if (!window) return null;
    const hour = Math.floor(window.lastHour);
    // Rows hold still between a design's own hours, and whether designs are known only at a recorded gap's edge.
    return data ? `${designsSince(data, hour)}:${designsKnown(data, hour)}` : `${hour}`;
  });
  const hour = useMemo(() => {
    const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
    return window ? Math.floor(window.lastHour) : null;
    // `shown` changes exactly when the rows may.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown, track, widget.time]);
  useShownEachFrame(runtime.clock, () => {
    const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
    return window ? Math.floor(window.lastHour) : null;
  });
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  const includedText = typeof options.included === 'string' ? options.included : '';
  const included = useMemo(() => { const keys = contentSet(includedText).map(contentKey); return keys.length ? new Set(keys) : null; }, [includedText]);
  const rows = useMemo(() => data && hour !== null ? designsAt(data, hour, included) : NO_ROWS, [data, hour, included]);
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => tag && hour !== null ? [{ hour, tag }] : [], [tag, hour]), active);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  // The sheet belongs to the recording, branch and country its design was picked from.
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag]);
  const owner = useMemo(() => ({ client: runtime.client, scope }), [runtime.client, scope]);
  const [stored, setPicked] = useState<{ owner: object; row: DesignRow } | null>(null);
  const picked = stored?.owner === owner ? rows.find(row => row.key === stored.row.key) ?? stored.row : null;
  useEffect(() => dispatch({ type: 'close' }), [owner]);
  const current = useMemo(() => rows.filter(row => !row.obsolete), [rows]);
  const obsolete = useMemo(() => rows.filter(row => row.obsolete), [rows]);
  const columns = useCallback((withObsolete: boolean): readonly TableColumn<DesignRow>[] => [
    { key: 'design', header: 'Design', label: 'Design', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? UNNAMED,
      render: row => <span className="design-name">{row.icon ? <img src={row.icon} alt="" /> : <ArtFallback icon="layers" />}
        <span><b>{row.name ?? UNNAMED}</b>{row.type && <small> {row.type}</small>}</span></span> },
    { key: 'firstSeen', header: 'First seen', label: 'First seen', numeric: true, width: 'max-content', renderMissing: true, priority: withObsolete ? 1 : undefined,
      value: row => row.firstSeen.hour, text: row => seenText(row.firstSeen, dayLabel), render: row => seenText(row.firstSeen, dayLabel) },
    ...withObsolete ? [{ key: 'seenObsolete', header: 'Seen obsolete', label: 'Seen obsolete', numeric: true, width: 'max-content', renderMissing: true,
      value: (row: DesignRow) => row.seenObsolete?.hour ?? null, text: (row: DesignRow) => seenText(row.seenObsolete, dayLabel),
      render: (row: DesignRow) => seenText(row.seenObsolete, dayLabel) }] : [],
  ], []);
  const currentColumns = useMemo(() => columns(false), [columns]);
  const obsoleteColumns = useMemo(() => columns(true), [columns]);
  const tooltip = useCallback((row: DesignRow) => fitTooltip([{ title: row.name ?? UNNAMED, subtitle: row.type ?? undefined, rows: [
    { label: 'First seen', value: seenText(row.firstSeen, dayLabel) },
    ...seenNote(row.firstSeen) ? [{ label: seenNote(row.firstSeen)!, tone: 'muted' as const }] : [],
    ...row.obsolete ? [{ label: 'Seen obsolete', value: seenText(row.seenObsolete, dayLabel) },
      ...seenNote(row.seenObsolete) ? [{ label: seenNote(row.seenObsolete)!, tone: 'muted' as const }] : []] : [],
    ...row.design.name.nameUnknown ? [{ label: row.design.name.nameUnknown, tone: 'muted' as const }] : [],
    { label: 'Equipment type', value: row.design.name.token || 'Equipment type unavailable', tone: 'muted' as const, lookupKey: row.design.name.token },
  ] }], viewport), [viewport]);
  const menu = useCallback((row: DesignRow) => track ? [row.firstSeen, row.seenObsolete].flatMap(seen => seen ? [jumpItem(runtime, track.id, seen.hour)] : []) : [], [runtime, track]);
  const open = (row: DesignRow) => {
    if (hour === null) return;
    setPicked({ owner, row });
    dispatch({ type: 'open', breakdown: SHEET.id, hour, playing: runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  };
  const at = hour === null ? null : drillHour(drill, hour) ?? hour;

  if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see designs." />;
  if (hour === null) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading designs…" />;
  const table = (label: string, list: readonly DesignRow[], tableColumns: readonly TableColumn<DesignRow>[], flagged: boolean) => <Table rowRevision={designRevision}
    key={`${track.source.id}:${track.branch.id}:${label}`} label={label} rows={list} columns={tableColumns} rowKey={row => row.key} sort={flagged ? OBSOLETE_SORT : sort}
    setOption={flagged ? undefined : setOption} cellBars={false} density={options.density === 'compact' ? 'compact' : 'comfortable'}
    rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)} narrow={narrowTableOf(options)} hoursPerSecond={hoursPerSecond} onRowClick={open} menu={menu} tooltip={tooltip} />;
  const country = nameReadText(names.get(nameKey(hour, tag)));
  const header = headerSlot && createPortal(<span className="pill">{current.length.toLocaleString()} current</span>, headerSlot);
  const main = !rows.length ? designsKnown(data, hour) ? <WidgetState kind="empty" icon="layers" message={`${country} has no saved designs yet.`} />
    : <WidgetState kind="unavailable" message="Designs were not recorded at this date." />
    : <div className="equipment-designs-body">
      {current.length ? <div className="equipment-designs-table" style={{ height: Math.min(current.length * 44 + 32, Math.max(160, viewport * 0.6)) }}>
        {table('Current designs', current, currentColumns, false)}</div> : <p className="equipment-designs-empty">No current designs.</p>}
      {obsolete.length > 0 && <details className="equipment-designs-obsolete">
        <summary>Obsolete designs ({obsolete.length.toLocaleString()})</summary>
        <div className="equipment-designs-table" style={{ height: Math.min(obsolete.length * 44 + 32, Math.max(160, viewport * 0.6)) }}>
          {table('Obsolete designs', obsolete, obsoleteColumns, true)}</div>
      </details>}
    </div>;
  return <div ref={anchor} data-detail={failureDetails([...names.values()].map(nameReadFailure))} className="equipment-designs">
    {header}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!picked} anchor={anchor}
      title={picked ? picked.name ?? UNNAMED : SHEET.label} day={dayLabel(at ?? hour)} following={drill.follow} canFollow={widget.time.mode === 'follow'}
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}>
      {picked && <DesignSheet />}
    </DrillHost>
  </div>;
}

/** The design's stat sheet at the drill's hour: main's `equipment-design-detail` product and its sections, which come
 * back with the equipment designs detail family (not yet in this build). */
function DesignSheet() {
  return <WidgetState kind="unavailable" message="The design sheet comes with the equipment design detail, which is not in this build yet." />;
}
