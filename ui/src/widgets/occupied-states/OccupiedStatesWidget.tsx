import { useCallback, useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import Table, { CellBar, CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { jumpItem } from '../../charts/jump';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilter } from '../../dashboard/groups';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { OccupiedStatesReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { countryHour, garrisonCell, occupiedAt, occupationTooltip, percentCell, stateName, validateOccupiedStatesReply } from './occupiedStatesData';
import type { OccupiedRow } from './occupiedStatesData';
import './occupiedStates.css';

const EMPTY_WARNINGS: string[] = [];
const EMPTY_ROWS: OccupiedRow[] = [];
const rowKey = (row: OccupiedRow) => row.key;

export default function OccupiedStatesWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const viewportHeight = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'occupied-states', params: { branchId: track.branch.id } } : null;
  const { data, error, retry } = useProduct<OccupiedStatesReply>(runtime.client, request, active, validateOccupiedStatesReply);
  const selection = widget.countries;
  const { data: groups, error: groupError, retry: groupRetry } = useCountryGroups(runtime, selection.kind === 'all' ? undefined : track, active);
  const identities = useMemo(() => groups ? countryIdentityIndex(groups.countryIdentities) : null, [groups]);
  const fallback = defaultCountryTag(track);
  const filter = useMemo(() => worldFilter(selection, fallback ? [fallback] : [], groups, hour, track?.branch.lastHour ?? 0),
    [selection, fallback, groups, hour, track?.branch.lastHour]);
  const matches = useMemo(() => filter.countryIds == null ? null : (tag: string, readHour: number) => {
    const key = identities?.keyAt(tag, readHour);
    return key != null && filter.countryIds!.has(key);
  }, [filter.countryIds, identities]);
  const identityKnown = useCallback((tag: string, readHour: number) => identities?.keyAt(tag, readHour) != null, [identities]);
  const view = useMemo(() => data ? occupiedAt(data, hour, matches, options.countryRole === 'occupied' ? 'occupied' : 'occupier', identityKnown) : null,
    [data, hour, matches, options.countryRole, identityKnown]);
  const rows = view?.rows ?? EMPTY_ROWS;
  const [visible, setVisible] = useState<readonly OccupiedRow[]>(EMPTY_ROWS);
  const onVisible = useCallback((rows: readonly OccupiedRow[]) => setVisible(rows), []);
  const wanted = useMemo(() => visible.flatMap(row => (['owner', 'occupier', 'occupied'] as const)
    .flatMap(role => row.value[role] === null ? [] : [{ hour: countryHour(row, role, hour), tag: row.value[role]! }])), [visible, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const sortingNames = useMemo(() => sort.column === 'owner' || sort.column === 'occupier'
    ? rows.flatMap(row => (['owner', 'occupier'] as const).flatMap(role => row.value[role] === null ? []
      : [{ hour: countryHour(row, role, hour), tag: row.value[role]! }])) : [], [rows, sort.column, hour]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const laws = useMemo(() => new Map(data?.laws.map(law => [law.token, law])), [data]);
  const columns = useMemo<readonly TableColumn<OccupiedRow>[]>(() => {
    const country = (role: 'owner' | 'occupier', label: string): TableColumn<OccupiedRow> => ({ key: role, header: label, label, renderMissing: true, width: 'minmax(150px, 1fr)',
      value: row => { const tag = row.value[role]; return tag === null ? null : names.get(nameKey(countryHour(row, role, hour), tag)) ?? tag; },
      render: row => { const tag = row.value[role]; return tag === null ? 'Unknown' : <CountryCell {...labelAt(countryHour(row, role, hour))(tag)} />; } });
    return [
      { key: 'state', header: 'State', label: 'State', renderMissing: true, width: 'minmax(160px, 1.2fr)', value: row => row.value.name ?? null, render: stateName },
      country('owner', 'Owner'), country('occupier', 'Occupier'),
      { key: 'resistance', header: 'Resistance', label: 'Resistance', numeric: true, bar: 10000000n, width: '150px', value: row => BigInt(row.value.resistanceRaw), render: row => percentCell(row.value.resistanceRaw) },
      { key: 'compliance', header: 'Compliance', label: 'Compliance', numeric: true, bar: 10000000n, width: '150px', value: row => BigInt(row.value.complianceRaw), render: row => percentCell(row.value.complianceRaw) },
      { key: 'garrison', header: 'Garrison', label: 'Garrison', numeric: true, renderMissing: true, width: '175px',
        value: row => row.value.garrisonRequiredRaw === null ? null : BigInt(row.value.garrisonRequiredRaw),
        render: row => { const cell = garrisonCell(row.value, options.cellBars === true);
          return <CellBar value={cell.bar} maximum={100000n}><span className={cell.red ? 'occupation-garrison-alert' : undefined}>{cell.text}</span></CellBar>; } },
      { key: 'law', header: 'Occupation law', label: 'Occupation law', renderMissing: true, width: 'minmax(200px, 1.2fr)', value: row => row.value.law === null ? null : laws.get(row.value.law)?.name ?? row.value.law,
        render: row => { const token = row.value.law; const law = token === null ? undefined : laws.get(token);
          return <span className="occupation-law">{law?.icon ? <img src={law.icon} alt="" /> : <i aria-label="Occupation law icon unavailable" />}<span>{token === null ? 'Not in this recording' : law?.name ?? (token || 'Unnamed law')}</span></span>; } },
    ];
  }, [names, labelAt, laws, options.cellBars, hour]);
  const tooltip = useCallback((row: OccupiedRow) => {
    if (!data) return { title: '', rows: [] };
    const content = occupationTooltip(data, row, hour, tag => labelAt(row.hour)(tag).name);
    const law = row.value.law === null ? undefined : laws.get(row.value.law);
    if (law?.missing) content.rows = [...content.rows, { label: law.missing, tone: 'muted' }];
    return fitTooltip([content], viewportHeight);
  }, [data, hour, labelAt, laws, viewportHeight]);
  useEffect(() => {
    onDiagnostics(data?.warnings ?? EMPTY_WARNINGS);
    return () => onDiagnostics(EMPTY_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" message="Load a campaign to see occupied states." />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (groupError) return <WidgetState kind="error" message={groupError} onRetry={groupRetry} />;
  if (!data || (selection.kind !== 'all' && !groups)) return <WidgetState kind="loading" label="Loading occupied states…" />;
  if (filter.reason) return <WidgetState kind="unavailable" message={filter.reason} />;
  if (!view?.observation) return <WidgetState kind="unavailable" message="The recording cannot establish occupied states at this moment. Gaps stay gaps." />;
  if (!rows.length && view.selectionUnknown) return <WidgetState kind="unavailable" message="Some occupied countries' native identities were not recorded at the supplying read; matching occupied states cannot be established." />;
  if (!rows.length) return <WidgetState kind={view.observation.complete ? 'empty' : 'unavailable'} message={view.observation.complete ? 'No occupied states match these countries at this moment.' : 'The occupied-state inventory is incomplete at this moment.'} />;
  return <div className="occupied-states">
    {!view.observation.complete && <p className="occupation-note">Some occupied states were not recorded.</p>}
    {view.selectionUnknown && <p className="occupation-note">Some occupied states could not be matched because their countries' native identities were not recorded at the supplying read.</p>}
    <Table label="Occupied states" rows={rows} rowKey={rowKey} columns={columns} sort={sort} setOption={setOption}
      cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true} density={options.density === 'compact' ? 'compact' : 'comfortable'}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} tooltip={tooltip} onVisibleRows={onVisible} hoursPerSecond={hoursPerSecond} />
  </div>;
}
