import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import CanvasTable from '../../charts/canvasTable/CanvasTable';
import { jumpItem } from '../../charts/jump';
import { tableSortOf, tableSortToken } from '../../charts/table/options';
import { barFraction, nextSort, sortRows } from '../../charts/table/logic';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { GameScoreReply } from '../../generated/dashboard';
import { useLiveMoments } from '../../live/useLiveMoments';
import type { PaintCell } from '../../charts/canvasTable/paint';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { FALLBACK_LABELS, SCORE_PARTS, formatScore, gameScoreCountryKnown, gameScoreLabel, gameScoreTooltips, scoreValue, validateGameScoreReply } from './gameScoreData';
import type { ScoreRow } from './gameScoreData';
import { liveScoreHour } from './liveScoreHour';

/** The Game score table of every country, its rows painted in a canvas (`CanvasTable`) from
 * hours read ahead of the playhead: nothing in it renders as playback moves. */
export default function CanvasGameScore({ widget, runtime, active, setOption }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const content = useContentRevision(runtime.client);
  const viewport = useViewportHeight();
  const labelsRequest = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'game_score', params: { branchId: track.branch.id } } : null;
  const labels = useProduct<GameScoreReply>(runtime.client, labelsRequest, active, validateGameScoreReply).data?.labels ?? FALLBACK_LABELS;
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const bars = options.cellBars === true;
  const density = options.density === 'compact' ? 'compact' : 'comfortable';
  const lane = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content, sort, bars]) : null;
  const { store, phase } = useLiveMoments<{ row: ScoreRow; reply: GameScoreReply }>({ active, lane, runtime, widget, track, content,
    read: async (hour, signal, restart) => {
      const { prepared, rows: ranked, through, partial } = await liveScoreHour(runtime, track!, content, hour, signal, restart);
      const rows = sort.column === 'total' && sort.direction === 'descending' ? ranked
        : sortRows(ranked, row => scoreValue(row, sort.column), sort.direction, row => row.key, row => row.tag);
      const maxima = bars ? SCORE_PARTS.map(part => rows.reduce((most, row) => { const raw = row.cells[part].raw; const value = raw == null ? 0n : BigInt(raw); return value > most ? value : most; }, 0n)) : null;
      return { through, partial, rows: rows.map((row, slot) => {
        // Formatted when first painted: of ~1,500 countries only the rows in view are.
        let cells: PaintCell[] | undefined;
        return { key: row.key, hour: row.hour, rank: slot + 1,
          data: { row, reply: prepared.reply },
          ...(gameScoreCountryKnown(row.identity) ? { tag: row.tag } : {}), name: row.identity.name ?? null,
          get cells() {
            return cells ??= SCORE_PARTS.map((part, at) => {
              const raw = row.cells[part].raw;
              return { text: formatScore(raw), bar: maxima && raw != null && maxima[at] > 0n ? barFraction(BigInt(raw), maxima[at]) : null };
            });
          } };
      }) };
    } });
  const columns = useMemo(() => [{ key: 'country', label: gameScoreLabel(labels, 'country'), kind: 'country' as const },
    ...SCORE_PARTS.map(part => ({ key: part, label: gameScoreLabel(labels, part), kind: 'numeric' as const }))], [labels]);
  const onSort = useCallback((key: string) => setOption?.('tableSort', tableSortToken(nextSort(sort, key, key === 'country' ? 'ascending' : 'descending'))), [sort, setOption]);

  if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to see its game scores." />;
  return <>
    {phase.state === 'error' && <WidgetState kind="error" failure={phase.failure} onRetry={store?.retry} />}
    {(phase.state === 'loading' || !store) && <WidgetState kind="loading" label="Loading game scores…" />}
    {phase.state === 'empty' && <WidgetState kind="unavailable" message="No country data at this time." />}
    {phase.state === 'unavailable' && <WidgetState kind="unavailable" message={phase.message} />}
    {store && <div hidden={phase.state !== 'ready'} style={{ display: phase.state === 'ready' ? 'contents' : undefined }}>
      <CanvasTable label="Game score" columns={columns} rank rankMarkers={options.rankMarkers === true} density={density} clock={runtime.clock} active={active}
        moment={store.moment} bind={store.bind} sort={sort} onSort={setOption ? onSort : undefined}
        tooltip={({ row, reply }) => fitTooltip([...gameScoreTooltips(reply, row)], viewport, { keep: SCORE_PARTS.length })}
        menu={({ row }) => [jumpItem(runtime, track.id, row.hour)]} />
    </div>}
  </>;
}
