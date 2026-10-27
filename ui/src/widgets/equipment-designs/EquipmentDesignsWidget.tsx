import { createPortal } from 'react-dom';
import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { preparedAt, variantDecimal } from '../../dashboard/economyHistory';
import { EquipmentCell, EQUIPMENT_UNNAMED, useVariantArt } from '../../dashboard/EquipmentCell';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { DesignDetailReply, ProductionCatalogueReply } from '../../generated/dashboard';
import { DesignSections, validateDesignDetail } from '../production-lines/EquipmentDesignDetail';
import { PRODUCTION_CATALOGUE } from '../production-licences/licencesData';
import { DEFAULT_SORT, OPTIONS, SHEET } from './definition';
import { designsAt, designsKnown, designTypes, seenNote, seenText, validateDesigns } from './designsData';
import type { DesignRow } from './designsData';
import './equipmentDesigns.css';

const NO_ROWS: readonly DesignRow[] = [];
/** Obsolete designs read newest flagged first. */
const OBSOLETE_SORT = { column: 'seenObsolete', direction: 'descending' } as const;
const NO_WARNINGS: readonly string[] = [];

function useDesigns(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: PRODUCTION_CATALOGUE,
    params: { branchId: track.branch.id, view: 'designs', tags: [tag] } } : null;
  return { track, tag, ...useProduct<ProductionCatalogueReply>(runtime.client, request, active, validateDesigns, track?.source) };
}

/** The Included types setting lists the equipment types the country's designs use. */
export function useDesignChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useDesigns(widget, runtime, true);
  return { included: data ? designTypes(data).map(type => ({ value: setToken(type.value), label: type.label })) : [] };
}

/** One country's saved designs at the date shown: current ones, then the obsolete
 * ones in a closed disclosure, each with when it and its obsolete flag were first read. */
export default function EquipmentDesignsWidget({ widget, runtime, active, headerSlot, onDiagnostics, setOption }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const { track, tag, data, error, retry } = useDesigns(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  const includedText = typeof options.included === 'string' ? options.included : '';
  const included = useMemo(() => { const keys = contentSet(includedText).map(contentKey); return keys.length ? new Set(keys) : null; }, [includedText]);
  const rows = useMemo(() => data && tag ? designsAt(data, tag, hour, included) : NO_ROWS, [data, tag, hour, included]);
  const current = useMemo(() => rows.filter(row => !row.obsolete), [rows]);
  const obsolete = useMemo(() => rows.filter(row => row.obsolete), [rows]);
  const [visible, setVisible] = useState<readonly string[]>([]);
  const onVisibleRows = useCallback((shown: readonly DesignRow[]) => setVisible(previous => [...new Set([...previous.slice(-256), ...shown.map(row => row.key)])]), []);
  const art = useVariantArt(runtime, track, tag, hour, visible, active);
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => tag ? [{ hour, tag }] : [], [tag, hour]), active);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [picked, setPicked] = useState<DesignRow | null>(null);
  const columns = useCallback((withObsolete: boolean): readonly TableColumn<DesignRow>[] => [
    { key: 'design', header: 'Design', label: 'Design', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? EQUIPMENT_UNNAMED,
      render: row => <EquipmentCell name={row.name} detail={row.type} icon={art.get(row.key)} /> },
    // Beside Seen obsolete, First seen is the date that hides first as the card narrows.
    { key: 'firstSeen', header: 'First seen', label: 'First seen', numeric: true, width: 'max-content', renderMissing: true, priority: withObsolete ? 1 : undefined,
      value: row => row.firstSeen?.hour ?? null, text: row => seenText(row.firstSeen, dayLabel), render: row => seenText(row.firstSeen, dayLabel) },
    ...withObsolete ? [{ key: 'seenObsolete', header: 'Seen obsolete', label: 'Seen obsolete', numeric: true, width: 'max-content', renderMissing: true,
      value: (row: DesignRow) => row.seenObsolete?.hour ?? null, text: (row: DesignRow) => seenText(row.seenObsolete, dayLabel),
      render: (row: DesignRow) => seenText(row.seenObsolete, dayLabel) }] : [],
  ], [art]);
  const currentColumns = useMemo(() => columns(false), [columns]);
  const obsoleteColumns = useMemo(() => columns(true), [columns]);
  const tooltip = useCallback((row: DesignRow) => fitTooltip([{ title: row.name ?? EQUIPMENT_UNNAMED, subtitle: row.type ?? undefined, rows: [
    { label: 'First seen', value: seenText(row.firstSeen, dayLabel) },
    ...seenNote(row.firstSeen) ? [{ label: seenNote(row.firstSeen)!, tone: 'muted' as const }] : [],
    ...row.obsolete ? [{ label: 'Seen obsolete', value: seenText(row.seenObsolete, dayLabel) },
      ...seenNote(row.seenObsolete) ? [{ label: seenNote(row.seenObsolete)!, tone: 'muted' as const }] : []] : [],
    { label: 'Design', value: row.design.token || 'Equipment type unavailable', tone: 'muted' as const, lookupKey: row.design.token },
  ] }], viewport), [viewport]);
  const menu = useCallback((row: DesignRow) => track ? [row.firstSeen, row.seenObsolete].flatMap(seen => seen ? [jumpItem(runtime, track.id, seen.hour)] : []) : [], [runtime, track]);
  const open = (row: DesignRow) => {
    setPicked(row);
    dispatch({ type: 'open', breakdown: SHEET.id, hour, playing: runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  };
  const at = drillHour(drill, hour) ?? hour;

  if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see designs." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading designs…" />;
  if (!preparedAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  const country = names.get(nameKey(hour, tag)) ?? tag;
  const table = (label: string, list: readonly DesignRow[], columns: readonly TableColumn<DesignRow>[], obsolete: boolean) => <Table key={`${track.source.id}:${track.branch.id}:${label}`}
    label={label} rows={list} columns={columns} rowKey={row => row.key} sort={obsolete ? OBSOLETE_SORT : sort} setOption={obsolete ? undefined : setOption} cellBars={false}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)}
    narrow={narrowTableOf(options)} hoursPerSecond={hoursPerSecond} onRowClick={open} menu={menu} tooltip={tooltip} onVisibleRows={onVisibleRows} />;
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
  return <div ref={anchor} className="equipment-designs">
    {header}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!picked} anchor={anchor}
      title={picked ? picked.name ?? EQUIPMENT_UNNAMED : SHEET.label} day={dayLabel(at)} following={drill.follow} canFollow={widget.time.mode === 'follow'}
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}>
      {picked && <DesignSheet runtime={runtime} track={track} hour={drill.follow ? hour : at} variant={variantDecimal(picked.key)} active={active} />}
    </DrillHost>
  </div>;
}

/** Production lines' stat sheet of one saved design at the drill's hour. */
function DesignSheet({ runtime, track, hour, variant, active }: { runtime: WidgetRuntime; track: NonNullable<ReturnType<typeof useDesigns>['track']>; hour: number; variant: string; active: boolean }) {
  const request = { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-design-detail', params: {
    branchId: track.branch.id, hour: Math.floor(hour), variant, fromVariant: null, ship: null, wing: null, observer: null, fog: null, targetCountry: null } };
  const { data, error, retry } = useProduct<DesignDetailReply>(runtime.client, request, active, validateDesignDetail);
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the design…" />;
  return <div className="equipment-design-detail"><DesignSections sections={data.sections} /></div>;
}
