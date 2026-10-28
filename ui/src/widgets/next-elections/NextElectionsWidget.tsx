import { useCallback, useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { dayLabel } from '../../charts/draw';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { POLITICS_HISTORY, preparedThroughHour, readAt } from '../../dashboard/politicsHistory';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { PoliticsHistoryReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { daysText, ELECTION_VALUES, electionsAt, electionTooltip, electionWarnings, termText, validateElectionsReply } from './nextElectionsData';
import type { ElectionRow } from './nextElectionsData';
import './nextElections.css';

const NO_ROWS: readonly ElectionRow[] = [];
const NO_WARNINGS: string[] = [];
const rowKey = (row: ElectionRow) => row.key;
const tieKey = (row: ElectionRow) => row.tag;

/** The table's columns: each country by `label` (sorted by `sortName`), the hiding
 * columns' values in the row tooltip. */
export function electionColumns(sortName: (tag: string) => string, label: CountryLabel, cellBars: boolean): TableColumn<ElectionRow>[] {
  return [
    { key: 'country', header: 'Country', label: 'Country', width: 'minmax(var(--table-name-min), 2fr)',
      value: row => sortName(row.tag), render: row => <CountryCell {...label(row.tag)} /> },
    { key: 'term', header: 'Term', label: 'Term', numeric: true, renderMissing: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 1,
      value: ELECTION_VALUES.term, text: row => row.term === null ? null : termText(row.term),
      render: row => <span className="election-term"><span>{row.term === null ? '—' : termText(row.term)}</span>
        {cellBars && <ProgressMeter value={row.term} state="live" label="Term passed" />}</span> },
    { key: 'days', header: 'Days left', label: 'Days left', numeric: true, width: 'max-content', priority: 2, inTooltip: true,
      value: ELECTION_VALUES.days, render: row => daysText(row.hoursLeft!) },
    { key: 'next', header: 'Next election', label: 'Next election', numeric: true, width: 'max-content', priority: 3, inTooltip: true,
      value: ELECTION_VALUES.next, render: row => dayLabel(row.next!) },
  ];
}

export default function NextElectionsWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const inside = !!track && !!window && track.branch.firstHour <= hour && hour <= track.branch.lastHour;
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedTags(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  // The whole branch's schedules once; playback only looks them up locally.
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: POLITICS_HISTORY,
    params: { branchId: track.branch.id, mode: 'elections' } } : null;
  const { data, error, retry } = useProduct<PoliticsHistoryReply>(runtime.client, request, active, validateElectionsReply, track?.source);
  const prepared = !!data && preparedThroughHour(data, hour);
  const read = !!data && readAt(data.known, hour);
  const view = useMemo(() => data && prepared && read && match.ready && !match.reason ? electionsAt(data, hour, match.matches) : null,
    [data, prepared, read, hour, match]);
  const rows = view?.rows ?? NO_ROWS;
  const [visible, setVisible] = useState<readonly ElectionRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((rows: readonly ElectionRow[]) => setVisible(rows), []);
  const wanted = useMemo(() => visible.map(row => ({ hour, tag: row.tag })), [visible, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  // A name sort needs every name; flags still load only for mounted rows.
  const sortingNames = useMemo(() => sort.column === 'country' ? rows.map(row => ({ hour, tag: row.tag })) : [], [rows, sort.column, hour]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const columns = useMemo(() => electionColumns(tag => names.get(nameKey(hour, tag)) ?? 'Country name unavailable', labelAt(hour), options.cellBars === true),
    [names, labelAt, hour, options.cellBars]);
  const tooltip = useCallback((row: ElectionRow) => fitTooltip([electionTooltip(row, labelAt(hour)(row.tag))], viewport), [labelAt, hour, viewport]);
  const menu = useCallback((row: ElectionRow) => {
    if (!track || row.next === null) return [];
    const item = jumpItem(runtime, track.id, row.next);
    return item.disabled ? [] : [item];
  }, [runtime, track]);
  const warnings = useMemo(() => data ? electionWarnings(data, view) : NO_WARNINGS, [data, view]);
  useEffect(() => {
    onDiagnostics(warnings);
    return () => onDiagnostics(NO_WARNINGS);
  }, [warnings, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="ballot" message="Load a campaign to see elections." />;
  if (!window || !inside) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading elections…" /></>;
  if (!prepared) return <>{groupFailure}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  if (!read) return <>{groupFailure}<WidgetState kind="unavailable" message="Election schedules were not recorded at this date." /></>;
  if (!rows.length && !match.known(hour)) return groupFailure ?? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  // A country whose eligibility was not read may hold elections: no row is not proof of none.
  if (!rows.length && view?.unknown) return <>{groupFailure}<WidgetState kind="unavailable" message="Election eligibility was not recorded at this date." /></>;
  if (!rows.length) return <>{groupFailure}<WidgetState kind="empty" icon="ballot" message="No country holds elections at this date." /></>;
  return <div className="next-elections">{groupFailure}
    <Table key={`${track.source.id}:${track.branch.id}`} label="Next elections" rows={rows} columns={columns} rowKey={rowKey} tieKey={tieKey}
      sort={sort} setOption={setOption} cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)}
      menu={menu} tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} />
  </div>;
}
