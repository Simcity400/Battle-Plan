import { pendingReplyAt } from '../../dashboard/preparedPrefix';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles.ts';
import { useRef } from 'react';
import { useParityRows } from '../../dashboard/parityRows';
import { useCallback, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import Table, { CellBar, CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { jumpItem } from '../../charts/jump';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING, worldFilter } from '../../dashboard/groups';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useProductWindows, useWindowDisplay } from '../../dashboard/productWindows';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { OccupiedStatesManifestReply, OccupiedStatesReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { countryHour, garrisonCell, occupiedAt, occupiedWindows, occupationTooltip, percentCell, stateName, validateOccupiedStatesManifest,
  validateOccupiedStatesReply } from './occupiedStatesData';
import type { OccupiedRow } from './occupiedStatesData';
import './occupiedStates.css';

const EMPTY_WARNINGS: string[] = [];
const EMPTY_ROWS: OccupiedRow[] = [];
const rowKey = (row: OccupiedRow) => row.key;

/** One replay block of the branch at a time: the one holding the shown hour and, while
 * following, its neighbours, so playback and scrubbing into them never wait. A far jump
 * keeps the last shown hour until its block arrives. */
export default function OccupiedStatesWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const follow = widget.time.mode === 'follow';
  const playbackHour = useTableHour(runtime.clock, active && follow);
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const viewportHeight = useViewportHeight();
  const contentRevision = useContentRevision(runtime.client);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'occupied-states-manifest', params: { branchId: track.branch.id } } : null;
  const { data: manifest, error, retry } = useProduct<OccupiedStatesManifestReply>(runtime.client, request, active, validateOccupiedStatesManifest, track?.source);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, contentRevision]) : null;
  const { target, wanted } = occupiedWindows(manifest?.windows ?? [], hour, follow);
  const windows = useProductWindows<OccupiedStatesReply>(runtime.client, identity, index => ({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'occupied-states', params: { branchId: track!.branch.id, window: index } }), wanted, active && manifest !== undefined, validateOccupiedStatesReply, track?.source, true);
  const current = windows.data.get(target);
  const display = useWindowDisplay(identity, current, hour, runtime.client);
  const shown = display?.hour ?? hour;
  const selection = widget.countries;
  const { data: groups, error: groupError, retry: groupRetry } = useCountryGroups(runtime, selection.kind === 'all' ? undefined : track, active);
  const identities = useMemo(() => groups ? countryIdentityIndex(groups.countryIdentities) : null, [groups]);
  const fallback = defaultCountryTag(track);
  const filter = useMemo(() => worldFilter(selection, fallback ? [fallback] : [], groups, shown, track?.branch.lastHour ?? 0),
    [selection, fallback, groups, shown, track?.branch.lastHour]);
  const matches = useMemo(() => filter.countryIds == null ? null : (tag: string, readHour: number) => {
    const key = identities?.keyAt(tag, readHour);
    return key != null && filter.countryIds!.has(key);
  }, [filter.countryIds, identities]);
  const identityKnown = useCallback((tag: string, readHour: number) => identities?.keyAt(tag, readHour) != null, [identities]);
  // Before the first block nothing is recorded; a held earlier display never stands in.
  const view = useMemo(() => display && target >= 0 ? occupiedAt(display.data, shown, matches, options.countryRole === 'occupied' ? 'occupied' : 'occupier', identityKnown) : null,
    [display, target, shown, matches, options.countryRole, identityKnown]);
  const rows = view?.rows ?? EMPTY_ROWS;
  const parityRoot = useRef<HTMLDivElement>(null);
  useParityRows(parityRoot, () => ({ kind: 'windowed', label: 'Occupied states values', complete: !!current && !!manifest,
    total: rows.length, hour: shown, rows: [...rows] }));
  const [visible, setVisible] = useState<readonly OccupiedRow[]>(EMPTY_ROWS);
  const onVisible = useCallback((rows: readonly OccupiedRow[]) => setVisible(rows), []);
  const wantedLabels = useMemo(() => visible.flatMap(row => (['owner', 'occupier', 'occupied'] as const)
    .flatMap(role => row.value[role] === null ? [] : [{ hour: countryHour(row, role, shown), tag: row.value[role]! }])), [visible, shown]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wantedLabels, active);
  const sortingNames = useMemo(() => sort.column === 'owner' || sort.column === 'occupier'
    ? rows.flatMap(row => (['owner', 'occupier'] as const).flatMap(role => row.value[role] === null ? []
      : [{ hour: countryHour(row, role, shown), tag: row.value[role]! }])) : [], [rows, sort.column, shown]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  // A window answered before preparation reached it carries the laws its states
  // hold, which the manifest published so far may not list yet.
  const laws = useMemo(() => new Map([...manifest?.laws ?? [], ...display?.data.laws ?? []].map(law => [law.token, law])), [manifest, display?.data]);
  const columns = useMemo<readonly TableColumn<OccupiedRow>[]>(() => {
    const country = (role: 'owner' | 'occupier', label: string, priority?: number): TableColumn<OccupiedRow> => ({ key: role, header: label, label, renderMissing: true, width: 'minmax(150px, 1fr)', priority,
      value: row => { const tag = row.value[role]; return tag === null ? null : names.get(nameKey(countryHour(row, role, shown), tag)) ?? 'Country name unavailable'; },
      render: row => { const tag = row.value[role]; return tag === null ? 'Unknown' : <CountryCell {...labelAt(countryHour(row, role, shown))(tag)} />; } });
    return [
      { key: 'state', header: 'State', label: 'State', renderMissing: true, width: 'minmax(160px, 1.2fr)', value: row => row.value.name ?? null, render: stateName },
      country('owner', 'Owner', 1), country('occupier', 'Occupier'),
      { key: 'resistance', header: 'Resistance', label: 'Resistance', numeric: true, bar: 10000000n, width: '150px', value: row => BigInt(row.value.resistanceRaw), render: row => percentCell(row.value.resistanceRaw) },
      { key: 'compliance', header: 'Compliance', label: 'Compliance', numeric: true, bar: 10000000n, width: '150px', priority: 2, text: row => percentCell(row.value.complianceRaw), value: row => BigInt(row.value.complianceRaw), render: row => percentCell(row.value.complianceRaw) },
      { key: 'garrison', header: 'Garrison', label: 'Garrison', numeric: true, renderMissing: true, width: '175px', priority: 3, text: row => garrisonCell(row.value, false).text,
        value: row => row.value.garrisonRequiredRaw === null ? null : BigInt(row.value.garrisonRequiredRaw),
        render: row => { const cell = garrisonCell(row.value, options.cellBars === true);
          return <CellBar value={cell.bar} maximum={100000n}><span className={cell.red ? 'occupation-garrison-alert' : undefined}>{cell.text}</span></CellBar>; } },
      { key: 'law', header: 'Occupation law', label: 'Occupation law', renderMissing: true, width: 'minmax(200px, 1.2fr)', priority: 4,
        text: row => row.value.law === null ? 'Not in this recording' : contentTitle(laws.get(row.value.law)?.name, row.value.law, 'Occupation law name unavailable'), value: row => row.value.law === null ? null : contentTitle(laws.get(row.value.law)?.name, row.value.law, 'Occupation law name unavailable'),
        render: row => { const token = row.value.law; const law = token === null ? undefined : laws.get(token);
          return <span className="occupation-law" data-tooltip={contentTitleHint(token, law?.missing)}>{law?.icon ? <img src={law.icon} alt="" /> : null}<span>{token === null ? 'Not in this recording' : contentTitle(law?.name, token, 'Occupation law name unavailable')}</span></span>; } },
    ];
  }, [names, labelAt, laws, options.cellBars, shown]);
  const tooltip = useCallback((row: OccupiedRow) => {
    if (!manifest) return { title: '', rows: [] };
    const content = occupationTooltip(row, tag => labelAt(row.hour)(tag).name);
    return fitTooltip([content], viewportHeight);
  }, [labelAt, viewportHeight]);
  const diagnostics = useMemo(() => manifest ? [...new Set([...manifest.warnings, ...(display?.data.warnings ?? EMPTY_WARNINGS)])] : EMPTY_WARNINGS,
    [manifest, display?.data]);
  useDiagnostics(onDiagnostics, diagnostics);

  if (!track) return <WidgetState kind="empty" message="Load a campaign to see occupied states." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  const groupFailure = groupError ? <WidgetState kind="error" detail={groupError} onRetry={groupRetry} /> : null;
  if (groupFailure && !groups) return groupFailure;
  if (windows.errors.has(target)) return <>{groupFailure}<WidgetState kind="error" detail={windows.errors.get(target)!} onRetry={windows.retry} /></>;
  if (!manifest || (target >= 0 && !display) || (selection.kind !== 'all' && !groups)) return <>{groupFailure}<WidgetState kind="loading" label="Loading occupied states…" /></>;
  if (filter.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={filter.reason} /></>;
  if (pendingReplyAt(display?.data, shown)) return <>{groupFailure}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  if (!view?.observation) return <>{groupFailure}<WidgetState kind="unavailable" message="Occupied states weren’t recorded at this time." /></>;
  // Countries at an hour still being prepared are not known yet, nor matched: no row is not proof of none.
  if (!rows.length && !filter.known) return groupFailure ?? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (!rows.length && view.selectionUnknown) return <>{groupFailure}<WidgetState kind="unavailable" message="Occupied states unavailable for these countries." /></>;
  if (!rows.length) return <>{groupFailure}<WidgetState kind={view.observation.complete ? 'empty' : 'unavailable'} message={view.observation.complete ? 'No occupied states match these countries at this moment.' : 'Some occupied states are missing.'} /></>;
  return <div className="occupied-states" ref={parityRoot}>{groupFailure}
    {!current && <p className="occupation-note" role="status">Updating occupied states…</p>}
    {!view.observation.complete && <p className="occupation-note">Some occupied states are missing.</p>}
    {view.selectionUnknown && <p className="occupation-note">Some states couldn’t be matched to these countries.</p>}
    <Table label="Occupied states" rows={rows} rowKey={rowKey} columns={columns} sort={sort} setOption={setOption}
      cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} tooltip={tooltip} onVisibleRows={onVisible} hoursPerSecond={hoursPerSecond} />
  </div>;
}
