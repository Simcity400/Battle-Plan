import { nameReadText } from '../../dashboard/nameReads.ts';
import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import { usNumber } from '../../charts/numberFormat';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { preparedThroughHour, readAt } from '../../dashboard/politicsHistory';
import { loadTransientProduct, useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { PoliticsHistoryReply, PoliticsStateReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { deployedAirByTag, EXILE_VALUES, exilesAt, exileTooltip, exileWarnings, legitimacyShare, legitimacyText, manpowerText, validateExileDetails, validateExilesReply } from './governmentsInExileData';
import type { ExileRow } from './governmentsInExileData';
import { exilesRequest, shownHour } from '../../dashboard/widgetRequests';
import './governmentsInExile.css';

const NO_ROWS: readonly ExileRow[] = [];
const NO_WARNINGS: string[] = [];
const rowKey = (row: ExileRow) => row.key;
const tieKey = (row: ExileRow) => row.tag;

/** The table's columns: countries by `label` (sorted by `sortName`), deployed air crews
 * by `airOf`, the hiding columns' values in the row tooltip. */
export function exileColumns(sortName: (tag: string) => string, label: CountryLabel, cellBars: boolean,
  airOf: (row: ExileRow) => number | null | undefined): TableColumn<ExileRow>[] {
  return [
    { key: 'exile', header: 'Exile', label: 'Exile', width: 'minmax(var(--table-name-min), 2fr)',
      value: row => sortName(row.tag), text: row => label(row.tag).name, render: row => <CountryCell {...label(row.tag)} /> },
    { key: 'host', header: 'Host', label: 'Host', width: 'minmax(var(--table-name-min), 2fr)', renderMissing: true, priority: 1, inTooltip: true,
      value: row => row.host === null ? null : sortName(row.host), text: row => row.host === null ? null : label(row.host).name,
      render: row => row.host === null ? 'Not recorded' : <CountryCell {...label(row.host)} /> },
    { key: 'legitimacy', header: 'Legitimacy', label: 'Legitimacy', numeric: true, width: 'minmax(calc(var(--space-6) * 3), 1fr)', priority: 2, inTooltip: true,
      value: EXILE_VALUES.legitimacy,
      render: row => <span className="exile-legitimacy"><span>{legitimacyText(row.legitimacy!)}</span>
        {cellBars && <ProgressMeter value={legitimacyShare(row.legitimacy!)} state="live" label="Legitimacy" />}</span> },
    { key: 'manpower', header: 'Manpower', label: 'Manpower', numeric: true, width: 'max-content', priority: 3, inTooltip: true,
      value: EXILE_VALUES.manpower, render: manpowerText },
    { key: 'air', header: 'Air crews', label: 'Air crews', numeric: true, width: 'max-content', priority: 5, inTooltip: true,
      value: row => airOf(row), render: row => usNumber.format(airOf(row)!) },
    { key: 'leaders', header: 'Army leaders', label: 'Army leaders', numeric: true, width: 'max-content', priority: 4, inTooltip: true,
      value: EXILE_VALUES.leaders, render: row => usNumber.format(row.leaders!) },
  ];
}

/** Deployed air crews at the shown hour: one uncached request per shown hour, only while exiles are shown. */
function useDeployedAir(runtime: WidgetProps['runtime'], track: BoundTrack | undefined, hour: number, enabled: boolean) {
  const revision = useContentRevision(runtime.client);
  const request = track && enabled ? { sourceId: track.source.id, revision: track.source.revision, product: 'politics-state',
    params: { branchId: track.branch.id, hour, view: 'exiles' } } : null;
  const key = JSON.stringify([request, revision]);
  const [state, setState] = useState<{ client: WidgetProps['runtime']['client']; key: string; data?: PoliticsStateReply; error?: Failure }>();
  useEffect(() => {
    if (!request) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal).then(reply => {
      const data = validateExileDetails(reply, hour);
      if (!controller.signal.aborted) setState({ client: runtime.client, key, data });
    }).catch(error => {
      if (!controller.signal.aborted) setState({ client: runtime.client, key, error: asFailure(error) });
    });
    return () => controller.abort();
    // Identity includes the hour, branch, source revision and content binding.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime.client, key]);
  const current = state?.client === runtime.client && state.key === key ? state : undefined;
  const air = useMemo(() => current?.data ? deployedAirByTag(current.data) : null, [current?.data]);
  const warnings = useMemo(() => current?.error ? [`1 refused exile deployed air crews: ${current.error}`] : current?.data?.warnings ?? NO_WARNINGS,
    [current?.data, current?.error]);
  return { air, warnings };
}

export default function GovernmentsInExileWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = shownHour(track, window);
  const inside = !!track && !!window && track.branch.firstHour <= hour && hour <= track.branch.lastHour;
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedTags(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const request = exilesRequest(track, hour);
  const { data, error, retry } = useProduct<PoliticsHistoryReply>(runtime.client, request, active, validateExilesReply, track?.source);
  const prepared = !!data && preparedThroughHour(data, hour);
  const read = !!data && readAt(data.known, hour);
  const view = useMemo(() => data && prepared && read && match.ready && !match.reason ? exilesAt(data, hour, match.matches) : null,
    [data, prepared, read, hour, match]);
  const rows = view?.rows ?? NO_ROWS;
  const details = useDeployedAir(runtime, track, hour, active && inside && rows.length > 0);
  // Undefined while the hour detail loads or when it does not list the exile; null when it says not recorded.
  const airOf = useCallback((row: ExileRow) => details.air?.has(row.tag) ? details.air.get(row.tag) : undefined, [details.air]);
  const [visible, setVisible] = useState<readonly ExileRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((rows: readonly ExileRow[]) => setVisible(rows), []);
  const wanted = useMemo(() => visible.flatMap(row => [{ hour, tag: row.tag }, ...row.host === null ? [] : [{ hour, tag: row.host }]]), [visible, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  // A name sort needs every name; flags still load only for mounted rows.
  const sortingNames = useMemo(() => sort.column === 'exile' ? rows.map(row => ({ hour, tag: row.tag }))
    : sort.column === 'host' ? rows.flatMap(row => row.host === null ? [] : [{ hour, tag: row.host }]) : [], [rows, sort.column, hour]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const columns = useMemo(() => exileColumns(tag => nameReadText(names.get(nameKey(hour, tag))), labelAt(hour), options.cellBars === true, airOf),
    [names, labelAt, hour, options.cellBars, airOf]);
  const tooltip = useCallback((row: ExileRow) => fitTooltip([exileTooltip(row, hour, labelAt(hour)(row.tag),
    row.host === null ? null : labelAt(hour)(row.host), airOf(row))], viewport), [labelAt, hour, viewport, airOf]);
  const warnings = useMemo(() => data ? exileWarnings(data, view, details.warnings) : NO_WARNINGS, [data, view, details.warnings]);
  useEffect(() => {
    onDiagnostics(warnings);
    return () => onDiagnostics(NO_WARNINGS);
  }, [warnings, onDiagnostics]);

  let body;
  if (!track) body = <WidgetState kind="empty" icon="flag" message="Load a campaign to see governments in exile." />;
  else if (!window || !inside) body = <WidgetState kind="unavailable" message="This time is outside the recording." />;
  else if (error) body = <WidgetState kind="error" failure={error} onRetry={retry} />;
  else if (needsGroups && groups.error && !groups.data) body = <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  else if (match.reason) body = <WidgetState kind="unavailable" message={match.reason} />;
  else if (!data || !match.ready) body = <WidgetState kind="loading" label="Loading governments in exile…" />;
  else if (!prepared) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (!read) body = <WidgetState kind="unavailable" message="Exiles were not recorded at this date." />;
  else if (!rows.length) body = match.known(hour) ? <WidgetState kind="empty" icon="flag" message="No governments in exile at this date." />
    : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  else body = <Table key={`${track.source.id}:${track.branch.id}`} label="Governments in exile" rows={rows} columns={columns} rowKey={rowKey} tieKey={tieKey}
    sort={sort} setOption={setOption} cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)}
    tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} />;
  return <div className="governments-in-exile">
    {headerSlot && view && createPortal(<span className="pill exile-header-count">{usNumber.format(rows.length)} in exile</span>, headerSlot)}
    {needsGroups && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    {body}
  </div>;
}
