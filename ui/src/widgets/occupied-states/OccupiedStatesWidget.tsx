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
import { groupMembers, groupRuns, groupUnobserved } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { OccupiedStatesReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { garrisonCell, occupiedAt, occupationTooltip, percentCell, validateOccupiedStatesReply } from './occupiedStatesData';
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
  const { data: groups, error: groupError, retry: groupRetry } = useCountryGroups(runtime, selection.kind === 'group' ? track : undefined, active);
  const countries = useMemo(() => {
    if (selection.kind === 'all') return null;
    if (selection.kind === 'tags') return new Set(selection.tags);
    if (selection.kind === 'selected') { const tag = defaultCountryTag(track); return new Set(tag ? [tag] : []); }
    if (!groups || !track) return undefined;
    const runs = groupRuns(groups, selection.group);
    const members = runs && groupMembers(runs, selection.membership, hour, track.branch.lastHour, Infinity, groupUnobserved(groups, selection.group));
    return members ? new Set(members) : undefined;
  }, [selection, groups, track, hour]);
  const view = useMemo(() => data && countries !== undefined ? occupiedAt(data, hour, countries, options.countryRole === 'occupied' ? 'occupied' : 'occupier') : null,
    [data, hour, countries, options.countryRole]);
  const rows = view?.rows ?? EMPTY_ROWS;
  const [visible, setVisible] = useState<readonly OccupiedRow[]>(EMPTY_ROWS);
  const onVisible = useCallback((rows: readonly OccupiedRow[]) => setVisible(rows), []);
  const wanted = useMemo(() => [...new Set(visible.flatMap(row => [row.value.owner, row.value.occupier, row.value.occupied].filter((tag): tag is string => tag !== null)))].map(tag => ({ hour, tag })), [visible, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const sortingNames = useMemo(() => sort.column === 'owner' || sort.column === 'occupier'
    ? [...new Set(rows.flatMap(row => [row.value.owner, row.value.occupier].filter((tag): tag is string => tag !== null)))].map(tag => ({ hour, tag })) : [], [rows, hour, sort.column]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const laws = useMemo(() => new Map(data?.laws.map(law => [law.token, law])), [data]);
  const columns = useMemo<readonly TableColumn<OccupiedRow>[]>(() => {
    const country = (role: 'owner' | 'occupier', label: string): TableColumn<OccupiedRow> => ({ key: role, header: label, label, renderMissing: true, width: 'minmax(150px, 1fr)',
      value: row => { const tag = row.value[role]; return tag === null ? null : names.get(nameKey(hour, tag)) ?? tag; },
      render: row => { const tag = row.value[role]; return tag === null ? 'Unknown' : <CountryCell {...labelAt(hour)(tag)} />; } });
    return [
      { key: 'state', header: 'State', label: 'State', renderMissing: true, width: 'minmax(160px, 1.2fr)', value: row => row.value.name ?? null, render: row => row.value.name || `State ${row.state}` },
      country('owner', 'Owner'), country('occupier', 'Occupier'),
      { key: 'resistance', header: 'Resistance', label: 'Resistance', numeric: true, bar: 10000000n, width: '150px', value: row => BigInt(row.value.resistanceRaw), render: row => percentCell(row.value.resistanceRaw) },
      { key: 'compliance', header: 'Compliance', label: 'Compliance', numeric: true, bar: 10000000n, width: '150px', value: row => BigInt(row.value.complianceRaw), render: row => percentCell(row.value.complianceRaw) },
      { key: 'garrison', header: 'Garrison', label: 'Garrison', numeric: true, renderMissing: true, width: '175px',
        value: row => row.value.garrisonRequiredRaw === null ? null : BigInt(row.value.garrisonRequiredRaw),
        render: row => { const cell = garrisonCell(row.value, options.cellBars === true);
          return <CellBar value={cell.bar} maximum={10000000n}><span className={cell.red ? 'occupation-garrison-alert' : undefined}>{cell.text}</span></CellBar>; } },
      { key: 'law', header: 'Occupation law', label: 'Occupation law', renderMissing: true, width: 'minmax(200px, 1.2fr)', value: row => row.value.law === null ? null : laws.get(row.value.law)?.name ?? row.value.law,
        render: row => { const token = row.value.law; const law = token === null ? undefined : laws.get(token);
          return <span className="occupation-law">{law?.icon ? <img src={law.icon} alt="" /> : <i aria-label="Occupation law icon unavailable" />}<span>{token === null ? 'Not in this recording' : law?.name || token || 'Unnamed law'}</span></span>; } },
    ];
  }, [hour, names, labelAt, laws, options.cellBars]);
  const tooltip = useCallback((row: OccupiedRow) => {
    if (!data) return { title: '', rows: [] };
    const content = occupationTooltip(data, row, hour, tag => labelAt(hour)(tag).name);
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
  if (!data || (selection.kind === 'group' && !groups)) return <WidgetState kind="loading" label="Loading occupied states…" />;
  if (countries === undefined) return <WidgetState kind="unavailable" message="Country group membership was not recorded at this moment." />;
  if (!view?.observation) return <WidgetState kind="unavailable" message="The recording cannot establish occupied states at this moment. Gaps stay gaps." />;
  if (!rows.length) return <WidgetState kind={view.observation.complete ? 'empty' : 'unavailable'} message={view.observation.complete ? 'No occupied states match these countries at this moment.' : 'The occupied-state inventory is incomplete at this moment.'} />;
  return <div className="occupied-states">
    {!view.observation.complete && <p className="occupation-note">Some occupied states were not recorded.</p>}
    <Table label="Occupied states" rows={rows} rowKey={rowKey} columns={columns} sort={sort} setOption={setOption}
      cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true} density={options.density === 'compact' ? 'compact' : 'comfortable'}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} tooltip={tooltip} onVisibleRows={onVisible} hoursPerSecond={hoursPerSecond} />
  </div>;
}
