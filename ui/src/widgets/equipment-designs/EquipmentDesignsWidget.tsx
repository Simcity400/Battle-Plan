import { countryFromKey } from '../../dashboard/countryKey.ts';
import { createPortal } from 'react-dom';
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
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
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import type { WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { gameWindow, subscribeGameWindows, windowDayLabel, withWindowDiagnostics } from '../../dashboard/gameWindows';
import type { ShownWindow } from '../../dashboard/gameWindows';
import { DEFAULT_SORT, OPTIONS, SHEET } from './definition';
import { designRevision, designTypes, designsAt, designsKnown, designsSince, seenNote, seenText, validateDesigns } from './designsData';
import type { DesignRow } from './designsData';
import './equipmentDesigns.css';
import type { EquipmentDesignsReply } from '../../generated/dashboard';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_ROWS: readonly DesignRow[] = [];
const NO_WARNINGS: readonly string[] = [];
const OBSOLETE_SORT = { column: 'seenObsolete', direction: 'descending' } as const;
const UNNAMED = 'Design name unavailable';
const equipmentDesignerWindow = () => gameWindow('equipmentDesigner');

function useDesigns(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const request = track && tag ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'equipment-designs', params: { branchId: track.branch.id, country: countryFromKey(tag) } }) : null;
  return { track, tag, ...useProduct<EquipmentDesignsReply>(runtime.client, request, active, validateDesigns, track?.source) };
}

export function useDesignChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useDesigns(widget, runtime, true);
  return { included: data ? designTypes(data).map(type => ({ value: setToken(type.value), label: type.label })) : [] };
}

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
    return data ? `${designsSince(data, hour)}:${designsKnown(data, hour)}` : `${hour}`;
  });
  const hour = useMemo(() => {
    const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
    return window ? Math.floor(window.lastHour) : null;
  }, [shown, track, widget.time]);
  useShownEachFrame(runtime.clock, () => {
    const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
    return window ? Math.floor(window.lastHour) : null;
  });
  const [windowLines, setWindowLines] = useState<readonly string[]>(NO_WARNINGS);
  const onWindowDiagnostics = useCallback((lines: string[]) =>
    setWindowLines(previous => previous.length === lines.length && previous.every((line, index) => line === lines[index]) ? previous : lines), []);
  const [windowShown, setWindowShown] = useState<ShownWindow | null>(null);
  const Designer = useSyncExternalStore(subscribeGameWindows, equipmentDesignerWindow, equipmentDesignerWindow);
  const diagnostics = useMemo(() => withWindowDiagnostics(data?.warnings ?? NO_WARNINGS, windowLines), [data?.warnings, windowLines]);
  useDiagnostics(onDiagnostics, diagnostics);
  const includedText = typeof options.included === 'string' ? options.included : '';
  const included = useMemo(() => { const keys = contentSet(includedText).map(contentKey); return keys.length ? new Set(keys) : null; }, [includedText]);
  const rows = useMemo(() => data && hour !== null ? designsAt(data, hour, included) : NO_ROWS, [data, hour, included]);
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => tag && hour !== null ? [{ hour, tag }] : [], [tag, hour]), active);
  const anchor = useRef<HTMLDivElement>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
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
    dispatch({ type: 'open', breakdown: SHEET.id, hour, follow: drillControl.followsAt(hour) });
  };
  const drillRegion = drillControl.region;
  const at = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);

  if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see designs." />;
  if (hour === null) return <WidgetState kind="outside" />;
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
  const closeDrill = useCallback(() => {
    dispatch({ type: 'close' });
    setWindowLines(NO_WARNINGS);
    setWindowShown(null);
  }, [dispatch]);
  return <div ref={anchor} data-detail={failureDetails([...names.values()].map(nameReadFailure))} className="equipment-designs">
    {header}
    <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null && !!picked} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={picked ? picked.name ?? UNNAMED : SHEET.label} day={windowShown ? windowDayLabel(windowShown) : at === null ? '' : dayLabel(at)} canFollow
      onClose={closeDrill} main={main}>
      {picked && Designer && at === null && <WidgetState kind="outside" />}
      {picked && Designer && at !== null && <Designer
        selection={{
          kind: 'equipmentDesigner',
          sourceId: track.source.id,
          revision: track.source.revision,
          branchId: track.branch.id,
          hour: at,
          tag,
          variant: picked.key,
        }}
        active={active}
        client={runtime.client}
        onDiagnostics={onWindowDiagnostics}
        onShown={setWindowShown}
        onClose={closeDrill}
      />}
      {picked && !Designer && <DesignSheet />}
    </DrillHost>
  </div>;
}

function DesignSheet() {
  return <WidgetState kind="unavailable" message="The design sheet comes with the equipment design detail, which is not in this build yet." />;
}
