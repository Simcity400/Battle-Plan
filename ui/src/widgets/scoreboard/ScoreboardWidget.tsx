import { useCallback, useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { tableSortOf, tableSortToken } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import { countryAliasesAt } from '../../dashboard/countryIdentity';
import type { WidgetProps } from '../../dashboard/definitions';
import { rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { ScoreboardReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { formatCell, rankingSort, scoreboardModel, scoreboardRequest, scoreboardRows, scoreboardSort, scoreboardTooltip, selectedColumns, validateScoreboardReply } from './scoreboardData';
import type { ScoreboardRow } from './scoreboardData';
import './scoreboard.css';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly ScoreboardRow[] = [];
const rowKey = (row: ScoreboardRow) => row.key;
const tieKey = (row: ScoreboardRow) => row.tag;

export default function ScoreboardWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const registryRequest = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard',
    params: { branchId: track.branch.id, columns: [], tags: [], top: null } } : null;
  const registryResult = useProduct<ScoreboardReply>(runtime.client, registryRequest, active, validateScoreboardReply);
  const registry = registryResult.data?.columns ?? [];
  const shown = useMemo(() => selectedColumns(registry, options.columns, registryResult.data?.defaultColumns ?? []), [registryResult.data, options.columns]);
  const sort = scoreboardSort(registry, shown, tableSortOf(options.tableSort, DEFAULT_SORT), registryResult.data?.defaultSort ?? '');
  const ranking = rankingSort(registry, sort, options.topMeasure, registryResult.data?.defaultSort ?? '');
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = selectedTags(widget.countries, track);
  const fallbackKey = JSON.stringify(fallback);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallbackKey, groups.data, track?.branch.lastHour]);
  const request = track && registryResult.data && shown.length && !resolved.reason && (resolved.top != null || resolved.tags.length)
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard',
      params: scoreboardRequest(track.branch.id, resolved, shown, ranking.column, ranking.direction) } : null;
  const { data, error, retry } = useProduct<ScoreboardReply>(runtime.client, request, active, validateScoreboardReply);
  const model = useMemo(() => data ? scoreboardModel(data) : null, [data]);
  const rows = useMemo(() => {
    if (!model || (grouped && !groups.data)) return NO_ROWS;
    const candidates = resolved.top == null && resolved.membership === null
      ? countryAliasesAt(model.reply.series, resolved.tags, hour) : rankedCandidates(resolved, hour, model.tags);
    return scoreboardRows(model, candidates, hour, ranking.column, ranking.direction, resolved.top);
  }, [model, grouped, groups.data, resolved, hour, ranking.column, ranking.direction]);
  const [visible, setVisible] = useState<readonly ScoreboardRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((rows: readonly ScoreboardRow[]) => setVisible(rows), []);
  const wanted = useMemo(() => visible.map(row => ({ hour: row.hour, tag: row.tag })), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const nameWanted = useMemo(() => sort.column === 'country' ? rows.map(row => ({ hour: row.hour, tag: row.tag })) : [], [rows, sort.column]);
  const names = useCountryNamesAt(runtime.client, track, nameWanted, active);
  const columns = useMemo<readonly TableColumn<ScoreboardRow>[]>(() => [
    { key: 'country', header: 'Country', label: 'Country', width: 'minmax(0, 2fr)',
      hint: resolved.top != null ? 'Alphabetical order of the chosen numerical top N. Top N by is in Countries settings.' : undefined,
      value: row => names.get(nameKey(row.hour, row.tag)) ?? null, renderMissing: true,
      render: row => <CountryCell {...labelAt(row.hour)(row.tag)} /> },
    ...shown.map(column => ({ key: column.key, header: column.label, label: column.label, numeric: true, width: 'max-content', bar: 'visible' as const,
      hint: column.reason ?? column.note, value: (row: ScoreboardRow) => row.cells.get(column.key)?.amount,
      render: (row: ScoreboardRow) => {
        const cell = row.cells.get(column.key);
        return formatCell(column, cell?.amount ?? null, cell?.atLeast);
      } })),
  ], [shown, names, labelAt, resolved.top]);
  const tooltip = useCallback((row: ScoreboardRow) => fitTooltip([scoreboardTooltip(row, shown, labelAt(row.hour)(row.tag).name)], viewport), [shown, labelAt, viewport]);
  const setTableOption = useCallback<NonNullable<WidgetProps['setOption']>>((key, value) => {
    if (key === 'tableSort') {
      const next = tableSortOf(value, DEFAULT_SORT);
      const measure = next.column === 'country' ? sort.column : next.column;
      if (registry.some(column => column.key === measure && column.available)) setOption?.('topMeasure', setToken(measure));
    }
    setOption?.(key, value);
  }, [setOption, registryResult.data, sort.column]);
  useEffect(() => {
    const absent = contentSet(options.columns).map(contentKey).filter(key => !registry.some(column => column.key === key));
    const unknown = registryResult.data && absent.length
      ? [`${absent.length} selected ${absent.length === 1 ? 'column is' : 'columns are'} not in this recording: ${absent.join(', ')}.`] : [];
    const warnings = [...(data?.warnings ?? registryResult.data?.warnings ?? []), ...(groups.data?.warnings ?? []), ...unknown];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, registryResult.data, groups.data, options.columns, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to compare its countries." />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign branch." />;
  if (registryResult.error) return <WidgetState kind="error" message={registryResult.error} onRetry={registryResult.retry} />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (grouped && groups.error) return <WidgetState kind="error" message={`Country groups are unavailable: ${groups.error}`} onRetry={groups.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
  if (registryResult.data && !shown.length) return <WidgetState kind="unavailable" message="The selected columns are not in this recording. Choose columns in settings." />;
  if (!registryResult.data || !data || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading scoreboard…" />;
  if (!rows.length) return <WidgetState kind="unavailable" message="No countries were recorded for this selection at this hour. Gaps stay gaps." />;
  return <div className="scoreboard">
    <Table key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} label="Scoreboard" rows={rows} columns={columns}
      rowKey={rowKey} tieKey={tieKey} sort={sort} setOption={setOption ? setTableOption : undefined} sortOptionValue={tableSortToken}
      cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} />
  </div>;
}
