import { useCallback, useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import Table, { CellBar, CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { AgencyComparisonReply, AgencyRankField } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { agencyRequestParams, agencyRows, agencyTooltip, operativesCell, upgradesCell, validateAgencyComparisonReply } from './agencyData';
import type { AgencyRow } from './agencyData';
import './agencyComparison.css';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly AgencyRow[] = [];
const rowKey = (row: AgencyRow) => row.key;

export default function AgencyComparisonWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window?.lastHour ?? track?.branch.lastHour ?? 0;
  const top = widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? widget.countries.group.count : null;
  const topField: AgencyRankField = options.topMeasure === 'active' || options.topMeasure === 'maximum' ? options.topMeasure : 'defense';
  const needsGroups = widget.countries.kind !== 'all' && top === null;
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedTags(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fallback = useMemo(() => selected, [selectedKey]);
  const params = track && agencyRequestParams(track.branch.id, widget.countries, fallback, groups.data, track.branch.lastHour, options.includeUncreated === true, topField);
  const request = track && params ? { sourceId: track.source.id, revision: track.source.revision, product: 'agency-comparison', params } : null;
  const { data, error, retry } = useProduct<AgencyComparisonReply>(runtime.client, request, active, validateAgencyComparisonReply);
  const filter = useMemo(() => top !== null ? { tags: null, reason: null }
    : worldFilter(widget.countries, fallback, groups.data, hour, track?.branch.lastHour ?? hour),
  [top, widget.countries, fallback, groups.data, hour, track?.branch.lastHour]);
  const rows = useMemo(() => data && (!needsGroups || groups.data) && !filter.reason ? agencyRows(data, hour, filter.tags) : NO_ROWS,
    [data, needsGroups, groups.data, filter, hour]);
  const [visible, setVisible] = useState<readonly AgencyRow[]>(NO_ROWS);
  const onVisible = useCallback((rows: readonly AgencyRow[]) => setVisible(rows), []);
  const wanted = useMemo(() => visible.map(row => ({ hour: row.hour, tag: row.tag })), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  // A name sort needs all names; flags are still requested only for mounted rows.
  const sortingNames = useMemo(() => sort.column === 'country' ? rows.map(row => ({ hour: row.hour, tag: row.tag })) : [], [rows, sort.column]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const columns = useMemo<readonly TableColumn<AgencyRow>[]>(() => [
    { key: 'country', header: 'Country', label: 'Country', width: 'minmax(0, 2fr)',
      value: row => names.get(nameKey(row.hour, row.tag)) ?? 'Country name unavailable',
      render: row => <CountryCell {...labelAt(row.hour)(row.tag)} /> },
    { key: 'agency', header: 'Agency', label: 'Agency', renderMissing: true, width: 'minmax(0, 2fr)',
      value: row => row.value.name, render: row => <span title={row.value.name ?? 'Unknown'}>{row.value.name ?? 'Unknown'}</span> },
    { key: 'upgrades', header: 'Agency upgrades', label: 'Agency upgrades', numeric: true, renderMissing: true, width: 'max-content',
      value: row => row.value.upgrades.nonnegativeLevelSum === null ? null : BigInt(row.value.upgrades.nonnegativeLevelSum),
      render: row => <span className={row.value.upgrades.nonnegativeLevelSum === null ? 'agency-unknown' : undefined}>{upgradesCell(row.value)}</span> },
    { key: 'defense', header: data?.defenseLabel ?? 'Defence', label: data?.defenseLabel ?? 'Defence', numeric: true, bar: 'visible', width: 'max-content',
      value: row => BigInt(row.value.defenseRaw), render: row => row.value.defenseValue ?? 'Unknown' },
    { key: 'active', header: 'Operatives active / maximum', label: 'Operatives active / maximum', numeric: true, width: 'max-content',
      value: row => row.value.activeCount,
      render: row => <CellBar value={options.cellBars === true && row.value.maximum > 0 ? row.value.activeCount : null} maximum={row.value.maximum}>{operativesCell(row.value)}</CellBar> },
  ], [names, labelAt, options.cellBars, data?.defenseLabel]);
  const tooltip = useCallback((row: AgencyRow) => fitTooltip([agencyTooltip(row, labelAt(row.hour)(row.tag).name)], viewport), [labelAt, viewport]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to compare its intelligence agencies." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (needsGroups && groups.error) return <WidgetState kind="error" detail={`Country identities are unavailable: ${groups.error}`} onRetry={groups.retry} />;
  if (filter.reason) return <WidgetState kind="unavailable" message={filter.reason} />;
  if (!data || (needsGroups && !groups.data)) return <WidgetState kind="loading" label="Loading intelligence agencies…" />;
  if (!data.available) return <WidgetState kind="unavailable" message="Intelligence agencies are not in this recording." />;
  if (!runAt(data.observed, hour)) return <WidgetState kind="unavailable" message="Agency values were not recorded at this time." />;
  if (!rows.length) return <WidgetState kind="empty" icon="shield" message="No recorded agencies match these countries and settings at this hour." />;
  return <div className="agency-comparison">
    <Table key={`${track.source.id}:${track.branch.id}`} label="Intelligence agencies" rows={rows} rowKey={rowKey} columns={columns} sort={sort} setOption={setOption}
      cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} muted={row => !row.value.created}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} tooltip={tooltip} onVisibleRows={onVisible} hoursPerSecond={hoursPerSecond} />
  </div>;
}
