import { useEffect, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries.ts';
import { int, object } from '../../dashboard/validation.ts';
import type { ScoreboardColumnKey, ScoreboardPreparedParams, ScoreboardPreparedReply } from '../../generated/dashboard';
import { scoreboardModel, scoreboardRows, validateScoreboardReply } from './scoreboardData.ts';
import type { ScoreboardRow } from './scoreboardData';

export function scoreboardRegistryRequest(branchId: string): ScoreboardPreparedParams {
  return { selection: { branchId, columns: [], tags: [], top: null }, hour: 0, sort: 'factories', descending: true, index: true, bars: false };
}
export function validatePreparedScoreboard(value: unknown): ScoreboardPreparedReply {
  if (!object(value) || !int(value.hour) || !object(value.maxima)) throw new Error('Invalid prepared scoreboard.');
  const data = validateScoreboardReply(value.data);
  if (Object.entries(value.maxima).some(([key, maximum]) => !data.columns.some(column => column.key === key)
    || typeof maximum !== 'string' || !/^\d{1,39}$/.test(maximum) || BigInt(maximum) >= (1n << 127n))
    || data.series.some(series => [...series.segments, ...series.identities,
      ...(series.membership ?? []), ...series.columns.flatMap(column => [...column.runs, ...column.readRuns])]
      .some(span => span.firstHour !== value.hour || span.lastHour !== value.hour))) throw new Error('Invalid prepared scoreboard moment.');
  return { hour: value.hour, data, maxima: value.maxima as ScoreboardPreparedReply['maxima'] };
}

/** Index values own sorting; a page supplies only this moment's mounted cells. */
export function preparedScoreboardRows(index: ScoreboardPreparedReply, page: ScoreboardPreparedReply | undefined,
  rank: ScoreboardColumnKey, descending: boolean): ScoreboardRow[] {
  const model = scoreboardModel(index.data);
  const rows = scoreboardRows(model, model.tags, index.hour, rank, descending ? 'descending' : 'ascending', undefined, true);
  if (!page || page.hour !== index.hour || page.data.branchId !== index.data.branchId) return rows;
  const cells = scoreboardModel(page.data);
  const reached = new Map(scoreboardRows(cells, cells.tags, page.hour, rank, descending ? 'descending' : 'ascending').map(row => [row.key, row]));
  return rows.map(row => {
    const exact = reached.get(row.key);
    return exact && exact.tag === row.tag ? { ...row, cells: new Map([...row.cells, ...exact.cells]) } : row;
  });
}

export function scoreboardCellPage(request: ScoreboardPreparedParams, tags: readonly string[]): ScoreboardPreparedParams {
  return { ...request, index: false, bars: false, selection: { ...request.selection, top: null, tags: [...new Set(tags)] } };
}
export async function queryScoreboard(client: DashboardClient, source: DashboardSource, request: ScoreboardPreparedParams, signal: AbortSignal, restart = false) {
  if (signal.aborted) throw new DOMException('Campaign preparation cancelled', 'AbortError');
  const result = validatePreparedScoreboard(await preparedSeriesQuery(client, source, 'scoreboard', request, signal, restart));
  if (result.hour !== request.hour || result.data.branchId !== request.selection.branchId) throw new Error('Prepared scoreboard belongs to another moment.');
  if (!request.index) {
    const supplied = new Set(result.data.series.map(row => row.tag));
    if (request.selection.tags.some(tag => !supplied.has(tag))) throw new Error('Prepared scoreboard did not supply the requested cells.');
  }
  return result;
}

/** Carry complete presented cells at their supplying moment while the next hour prepares. */
export function usePreparedScoreboard(client: DashboardClient, source: DashboardSource | undefined,
  request: ScoreboardPreparedParams | null, visible: readonly ScoreboardRow[], active: boolean) {
  const content = useContentRevision(client);
  const owner = JSON.stringify([source?.id, source?.revision, content, request && { ...request, hour: undefined }]);
  const moment = JSON.stringify(request);
  const [attempt, setAttempt] = useState(0);
  const [all, setAll] = useState(false);
  const [state, setState] = useState<{ client: DashboardClient; owner: string; index?: ScoreboardPreparedReply; page?: ScoreboardPreparedReply; pending: boolean; error?: string }>();
  const owned = state?.client === client && state.owner === owner ? state : undefined;
  useEffect(() => {
    if (!active || !source || !request) return;
    const controller = new AbortController();
    setState(previous => ({ client, owner, index: previous?.client === client && previous.owner === owner ? previous.index : undefined,
      page: previous?.client === client && previous.owner === owner ? previous.page : undefined, pending: true }));
    void (async () => {
      const index = await queryScoreboard(client, source, request, controller.signal, attempt > 0);
      const candidates = preparedScoreboardRows(index, undefined, request.sort, request.descending);
      // Keep supplying cells for mounted holders and the same number of new
      // leading ranks. The native table then reports its actual mounted rows.
      const previousKeys = new Set(visible.map(row => row.key));
      const tags = all ? candidates.map(row => row.tag) : [...new Set([
        ...candidates.filter(row => previousKeys.has(row.key)).map(row => row.tag),
        ...candidates.slice(0, Math.max(visible.length, 1)).map(row => row.tag),
      ])];
      const page = await queryScoreboard(client, source, scoreboardCellPage(request, tags), controller.signal, attempt > 0);
      if (!controller.signal.aborted) setState({ client, owner, index, page, pending: false });
    })().catch(error => { if (!controller.signal.aborted) setState(previous => ({ ...previous, client, owner, pending: false, error: String(error) })); });
    return () => controller.abort();
  }, [client, source?.id, source?.revision, owner, moment, active, attempt]);
  const shown = owned?.index;
  const wanted = shown && (all ? shown.data.series.map(row => row.tag) : visible.filter(row => row.hour === shown.hour).map(row => row.tag));
  const wantedKey = JSON.stringify(wanted ?? []);
  const loaded = new Set(owned?.page?.data.series.map(row => row.tag));
  const covered = wanted?.every(tag => loaded.has(tag)) ?? true;
  useEffect(() => {
    if (!active || !source || !request || !shown || owned?.pending || covered || !wanted?.length) return;
    const controller = new AbortController();
    const page = scoreboardCellPage({ ...request, hour: shown.hour }, wanted);
    void queryScoreboard(client, source, page, controller.signal, attempt > 0).then(page => {
      if (!controller.signal.aborted) setState(previous => previous?.client === client && previous.owner === owner && previous.index === shown ? { ...previous, page } : previous);
    }).catch(error => { if (!controller.signal.aborted) setState(previous => previous?.owner === owner ? { ...previous, error: String(error) } : previous); });
    return () => controller.abort();
  }, [client, owner, shown, wantedKey, active, covered, owned?.pending, attempt]);
  return { index: shown, page: owned?.page, pending: owned?.pending ?? (active && !!request), error: owned?.error,
    retry: () => setAttempt(value => value + 1), complete: !!shown && shown.data.series.every(row => loaded.has(row.tag)),
    prepare: () => setAll(true) };
}
