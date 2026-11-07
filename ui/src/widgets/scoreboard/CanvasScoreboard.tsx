import { useCallback, useMemo } from 'react';
import { asFailure } from '../../dashboard/queryError.ts';
import WidgetState from '../../components/WidgetState';
import CanvasTable from '../../charts/canvasTable/CanvasTable';
import { tableSortOf, tableSortToken } from '../../charts/table/options';
import { barFraction, nextSort } from '../../charts/table/logic';
import { fitTooltip } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useContentRevision } from '../../dashboard/contentRevision';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveCountries } from '../../dashboard/groups';
import { resolveOptions, setToken } from '../../dashboard/options';
import { finalPreparedReply } from '../../dashboard/preparedPrefix';
import { useProduct } from '../../dashboard/products';
import type { ScoreboardPreparedParams, ScoreboardPreparedReply } from '../../generated/dashboard';
import { useLiveMoments } from '../../live/useLiveMoments';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { formatCell, rankingSort, scoreboardRequest, scoreboardSort, scoreboardTooltip, selectedColumns } from './scoreboardData';
import type { ScoreboardRow } from './scoreboardData';
import { preparedScoreboardRows, queryScoreboard, scoreboardCellPage, scoreboardRegistryRequest, validatePreparedScoreboard } from './preparedScoreboard';
import './scoreboard.css';

/** The Scoreboard with its rows painted in a canvas (`CanvasTable`) from hours read ahead of the
 * playhead: nothing in it renders as playback moves. */
export default function CanvasScoreboard({ widget, runtime, active, setOption }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const content = useContentRevision(runtime.client);
  const viewport = useViewportHeight();
  const registryRequest = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard',
    params: scoreboardRegistryRequest(track.branch.id) } : null;
  const registryResult = useProduct<ScoreboardPreparedReply>(runtime.client, registryRequest, active, validatePreparedScoreboard, track?.source);
  const registryData = registryResult.data?.data;
  const registry = useMemo(() => registryData?.columns ?? [], [registryData]);
  const shown = useMemo(() => selectedColumns(registry, options.columns, registryData?.defaultColumns ?? []), [registry, options.columns, registryData]);
  const sort = scoreboardSort(registry, shown, tableSortOf(options.tableSort, DEFAULT_SORT), registryData?.defaultSort ?? '');
  const ranking = rankingSort(registry, sort, options.topMeasure, registryData?.defaultSort ?? '');
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const resolved = useMemo(() => resolveCountries(widget.countries, JSON.parse(fallbackKey), undefined, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallbackKey, track?.branch.lastHour]);
  const selection = track ? scoreboardRequest(track.branch.id, resolved, shown, ranking.column, ranking.direction) : null;
  const bars = options.cellBars === true;
  const density = options.density === 'compact' ? 'compact' : 'comfortable';
  const lane = track && registryData && shown.length && selection && !resolved.reason && (resolved.top != null || selection.tags.length)
    ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content, selection, ranking.column, ranking.direction, bars, sort]) : null;
  // The top N are chosen by the measure; a country sort orders them by name as named at their hour.
  const byName = sort.column === 'country' ? sort.direction : undefined;
  const { store, phase } = useLiveMoments<{ row: ScoreboardRow }>({ active, lane, runtime, widget, track, content, byName,
    read: async (hour, signal, restart) => {
      const request: ScoreboardPreparedParams = { selection: selection!, hour, sort: ranking.column, descending: ranking.direction === 'descending', index: true, bars };
      const index = await queryScoreboard(runtime.client, track!.source, request, signal, restart);
      const candidates = preparedScoreboardRows(index, undefined, ranking.column, ranking.direction === 'descending');
      const page = candidates.length ? await queryScoreboard(runtime.client, track!.source, scoreboardCellPage(request, candidates.map(row => row.tag)), signal, restart) : undefined;
      const rows = preparedScoreboardRows(index, page, ranking.column, ranking.direction === 'descending');
      const maxima = shown.map(column => BigInt(index.maxima[column.key] ?? '0'));
      return { through: hour, partial: !finalPreparedReply(index) || (page !== undefined && !finalPreparedReply(page)), rows: rows.map((row, slot) => ({ key: row.key, tag: row.tag, hour: row.hour, rank: slot + 1,
        data: { row },
        cells: shown.map((column, at) => {
          const cell = row.cells.get(column.key);
          return { text: cell && (cell.amount === null || cell.readHour !== null) ? formatCell(column, cell.amount, cell.atLeast) : '…',
            bar: bars && cell?.amount != null && maxima[at] > 0n ? barFraction(cell.amount, maxima[at]) : null };
        }) })) };
    } });
  const columns = useMemo(() => [{ key: 'country', label: 'Country', kind: 'country' as const },
    ...shown.map(column => ({ key: column.key, label: column.label, kind: 'numeric' as const }))], [shown]);
  const onSort = useCallback((key: string) => {
    const next = nextSort(sort, key, key === 'country' ? 'ascending' : 'descending');
    const measure = next.column === 'country' ? sort.column : next.column;
    if (registry.some(item => item.key === measure && item.available)) setOption?.('topMeasure', setToken(measure));
    setOption?.('tableSort', tableSortToken(next));
  }, [registry, sort, setOption]);

  if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to compare its countries." />;
  if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
  if (registryResult.error) return <WidgetState kind="error" failure={asFailure(registryResult.error)} onRetry={registryResult.retry} />;
  if (registryResult.data && !shown.length) return <WidgetState kind="unavailable" message="The selected columns are not in this recording. Choose columns in settings." />;
  return <div className="scoreboard">
    {phase.state === 'error' && <WidgetState kind="error" failure={phase.failure} onRetry={store?.retry} />}
    {(phase.state === 'loading' || !store) && <WidgetState kind="loading" label="Loading scoreboard…" />}
    {phase.state === 'empty' && <WidgetState kind="unavailable" message="No countries available for this selection." />}
    {phase.state === 'unavailable' && <WidgetState kind="unavailable" message={phase.message} />}
    {store && <div hidden={phase.state !== 'ready'} style={{ display: phase.state === 'ready' ? 'contents' : undefined }}>
      <CanvasTable label="Scoreboard" columns={columns} rank rankMarkers={options.rankMarkers === true} density={density} clock={runtime.clock} active={active}
        moment={store.moment} bind={store.bind} sort={sort} onSort={setOption ? onSort : undefined}
        tooltip={({ row }, painted) => fitTooltip([scoreboardTooltip(row, shown, painted.country?.name ?? row.tag)], viewport)} />
    </div>}
  </div>;
}
