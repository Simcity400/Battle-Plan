import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import { CountryFlagUrl } from '../../components/CountryFlag';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import { nameKey, useCountryFactsAt } from '../../dashboard/countryNamesAt';
import type { FactSpec } from '../../dashboard/countryFacts';
import { useContentRevision } from '../../dashboard/contentRevision';
import { drawableIcon } from '../../dashboard/artwork';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useFollowedProduct } from '../../dashboard/products';
import { useParityRows } from '../../dashboard/parityRows';
import { useWindowDisplay } from '../../dashboard/productWindows';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { AirAcePortrait } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS, SORT_CHOICES } from './definition';
import { aceJump, aceName, aceNameValue, aceCallsign, aceRowModel, aceRows, aceStatus, aceTooltip, aircraftName, validateAceRoster, validatePortraits, wingName } from './acesData';
import type { AceRow, AceRosterData } from './acesData';
import './aces.css';

const EMPTY: readonly AceRow[] = [];
const WARNINGS: string[] = [];
const PORTRAITS: FactSpec<AirAcePortrait> = {
  product: 'air-ace-portraits', perRequest: Infinity, shared: true,
  params: (branchId, hour, ids) => ({ branchId, hour, ids }),
  extract: value => new Map(validatePortraits(value).portraits.map(p => [p.id, { ...p, image: p.image && drawableIcon(p.image) ? p.image : null }])),
};
const keyOf = (row: AceRow) => row.key;
export default function AcesWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const hourAt = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const speed = usePlaybackMotion(runtime.clock);
  const viewport = useViewportHeight();
  const content = useContentRevision(runtime.client);
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const window = track && resolveWindow(widget, track, runtime.tracks, hourAt);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const options = resolveOptions(OPTIONS, widget.options);
  const loaded = useFollowedProduct<AceRosterData>(runtime.client, track ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'air-aces', params: { branchId: track.branch.id, view: 'roster', hour },
  } : null, ['hour'], active, validateAceRoster, track?.source);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : null;
  // A roster still shown while the next hour loads keeps the hour it answers.
  const answered = loaded.moment ? (JSON.parse(loaded.moment) as [number])[0] : hour;
  const display = useWindowDisplay(identity, loaded.data, answered, runtime.client);
  const data = display?.data.manifest, detail = display?.data.roster;
  const read = display?.data.readHour ?? null;
  const preparing = !!display?.data.preparing.length;
  const selectedSort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const sort = preparing && selectedSort.column === 'date' ? { column: 'name', direction: 'ascending' as const } : selectedSort;
  const pending = loaded.data === undefined;
  const recorded = data !== undefined && runAt(data.observed, display?.hour ?? hour) !== null;
  const rows = useMemo(() => data && detail && recorded && read !== null && match.ready ? aceRows(data, detail, Math.max(detail.firstHour, read),
    (tag, read) => tag === null ? widget.countries.kind === 'all' : match.matches(tag, read)).map(row => ({ ...row, preparing })) : EMPTY,
  [data, detail, recorded, read, match, widget.countries.kind, preparing]);
  const [visible, setVisible] = useState<readonly AceRow[]>(EMPTY);
  useEffect(() => setVisible(EMPTY), [identity]);
  const onVisible = useCallback((rows: readonly AceRow[]) => setVisible(rows), []);
  const countries = useMemo(() => visible.flatMap(row => [row.run.tag, row.run.death?.killerTag].flatMap(tag => tag ? [{ tag, hour: row.hour }] : [])), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, countries, active && data !== undefined);
  const artRequests = useMemo(() => visible.map(row => ({ tag: row.key, hour: row.hour })), [visible]);
  const portraits = useCountryFactsAt(runtime.client, track, artRequests, active && data !== undefined, PORTRAITS, new Set(artRequests.map(item => item.hour)).size);
  const columns = useMemo<readonly TableColumn<AceRow>[]>(() => [
    { key: 'portrait', header: null, label: 'Portrait', sortable: false, width: 'var(--military-portrait-column)', value: () => null, renderMissing: true,
      render: row => { const art = portraits.get(nameKey(row.hour, row.key)); return art?.image ? <img className="aces-portrait" src={art.image} alt="" />
        : <ArtFallback icon="helmet" className="aces-portrait" />; } },
    { ...SORT_CHOICES[1], label: 'Ace', header: 'Ace', width: 'minmax(var(--military-name-min), 1.6fr)', value: aceNameValue, renderMissing: true,
      render: row => { const label = row.run.tag ? labelAt(row.hour)(row.run.tag) : null; return <div className="aces-who"><b title={aceName(row)}>{aceName(row)}</b><small>
        {label ? <CountryFlagUrl className="aces-flag" url={label.flag} reason={label.flagReason} pending={label.flagPending} />
          : <span className="aces-flag aces-country-missing" role="img" aria-label="Country not recorded" data-tooltip="Country not recorded" />}
        {aceCallsign(row) ? `“${aceCallsign(row)}”` : ''}
      </small></div>; } },
    { key: 'wing', header: 'Wing · aircraft', label: 'Wing and aircraft', sortable: false, width: 'minmax(var(--table-name-min), 1fr)', priority: 2, inTooltip: true, value: () => null, renderMissing: true,
      render: row => <div className="aces-wing"><span title={wingName(row)}>{wingName(row)}</span><small title={aircraftName(row)}>{aircraftName(row)}</small></div> },
    { ...SORT_CHOICES[0], label: 'Status', header: 'Status', width: 'var(--military-status-column)', priority: 1, inTooltip: true, value: row => row.ace.firstSeen, renderMissing: true,
      render: row => <small className="aces-status">{aceStatus(row)}</small> },
  ], [portraits, labelAt]);
  const tooltip = useCallback((row: AceRow) => fitTooltip([aceTooltip(row, tag => labelAt(row.hour)(tag).name)], viewport), [labelAt, viewport]);
  const diagnostics = useMemo(() => data ? [...data.warnings, ...(detail?.warnings ?? WARNINGS)] : WARNINGS, [data, detail]);
  useDiagnostics(onDiagnostics, diagnostics);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'windowed', label: 'Ace roster values',
    complete: !!data && !!detail && match.ready && !loaded.error, total: rows.length,
    rows: rows.map(aceRowModel), read, hour: display?.hour ?? hour, window: detail?.window,
  }));
  if (!track) return <WidgetState kind="empty" icon="star" message="Load a campaign to see its aces." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (loaded.error && !display) return <WidgetState kind="error" detail={loaded.error} onRetry={loaded.retry} />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading aces…" /></>;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!data.available) return <>{groupFailure}<WidgetState kind="unavailable" message={preparing ? "Aces were not recorded at this time." : "Aces: Not in this recording."} /></>;
  if (!recorded) return <>{groupFailure}<WidgetState kind="unavailable" message="Aces were not recorded at this time." /></>;
  if (!display) return <>{groupFailure}<WidgetState kind="loading" label="Loading aces..." /></>;
  // Aces are matched at the hour read: a country whose membership there is still being prepared is not known to be left out.
  if (!rows.length && read !== null && !match.known(read)) return groupFailure ?? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (!rows.length) return <>{groupFailure}<WidgetState kind="empty" icon="star" message="No aces match these settings." /></>;
  return <div ref={root} className="aces-widget">{groupFailure}{preparing && <p className="chart-line-key" role="status">Ace and death dates are still being prepared. Current roster sorted by name.</p>}{(pending || loaded.error) && <div className="aces-loading" role="status">{loaded.error ? 'Couldn’t load aces.' : 'Updating…'}{loaded.error && <button onClick={loaded.retry}>Retry</button>}</div>}<Table key={identity} label="Aces" rows={rows} columns={columns} rowKey={keyOf} sort={sort} setOption={setOption}
    rowHeight={50} cellBars={false} rankMarkers={false} density="comfortable" narrow={narrowTableOf(options)}
    muted={row => !row.run.alive} menu={row => row.preparing ? [] : [jumpItem(runtime, track.id, aceJump(row))]} tooltip={tooltip} onVisibleRows={onVisible} hoursPerSecond={speed}
    {...(import.meta.env.MODE === 'parity' ? { parity: { complete: true, item: aceRowModel } } : {})} /></div>;
}
