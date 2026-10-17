import { fieldPreparing, preparedPrefix } from '../../dashboard/preparedPrefix.ts';
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
  preparedPrefix(value);
  if (value.preparing !== undefined && (!Array.isArray(value.preparing) || !value.preparing.every(key => data.columns.some(column => column.key === key)))) throw new Error('Invalid preparing scoreboard columns.');
  return { ...value, hour: value.hour, data, maxima: value.maxima as ScoreboardPreparedReply['maxima'] };
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
export async function queryScoreboard(client: DashboardClient, source: DashboardSource, request: ScoreboardPreparedParams, signal: AbortSignal, restart = false, publish?: (value: ScoreboardPreparedReply) => void) {
  if (signal.aborted) throw new DOMException('Campaign preparation cancelled', 'AbortError');
  const validate = (value: unknown) => {
    const result = validatePreparedScoreboard(value);
    if (result.hour !== request.hour || result.data.branchId !== request.selection.branchId) throw new Error('Prepared scoreboard belongs to another moment.');
    if (!request.index && !preparedPrefix(result)?.pending.length) {
      const supplied = new Set(result.data.series.map(row => row.tag));
      if (request.selection.tags.some(tag => !supplied.has(tag))) throw new Error('Prepared scoreboard did not supply the requested cells.');
    }
    return result;
  };
  const result = validate(await preparedSeriesQuery(client, source, 'scoreboard', request, signal, restart, undefined, undefined,
    publish ? value => publish(validate(value)) : undefined));
  return result;
}

/** Publish each supplied index/cell prefix and retain the previous supplying moment while a new hour loads. */
export function usePreparedScoreboard(client: DashboardClient, source: DashboardSource | undefined,
  request: ScoreboardPreparedParams | null, visible: readonly ScoreboardRow[], active: boolean) {
  const content = useContentRevision(client);
  const owner = JSON.stringify([source?.id, source?.revision, content, request && { ...request, hour: undefined }]);
  const moment = JSON.stringify(request);
  const [attempt, setAttempt] = useState(0);
  const [all, setAll] = useState(false);
  const [state, setState] = useState<{ client: DashboardClient; owner: string; index?: ScoreboardPreparedReply; page?: ScoreboardPreparedReply; pending: boolean; error?: string }>();
  const [cellFailure, setCellFailure] = useState<{ client: DashboardClient; owner: string; key: string; attempt: number; error: string }>();
  const owned = state?.client === client && state.owner === owner ? state : undefined;
  useEffect(() => {
    if (!active || !source || !request) return;
    const controller = new AbortController();
    setState(previous => ({ client, owner, index: previous?.client === client && previous.owner === owner ? previous.index : undefined,
      page: previous?.client === client && previous.owner === owner ? previous.page : undefined, pending: true }));
    void queryScoreboard(client, source, request, controller.signal, attempt > 0, index => {
      if (!controller.signal.aborted) setState(previous => ({ client, owner, index,
        page: previous?.client === client && previous.owner === owner ? previous.page : undefined, pending: false }));
    }).catch(error => { if (!controller.signal.aborted) setState(previous => ({ ...previous, client, owner, pending: false, error: String(error) })); });
    return () => controller.abort();
  }, [client, source?.id, source?.revision, owner, moment, active, attempt]);
  const shown = owned?.index;
  const candidates = shown && request ? preparedScoreboardRows(shown, undefined, request.sort, request.descending) : [];
  const mounted = new Set(visible.filter(row => row.hour === shown?.hour).map(row => row.key));
  const previousKeys = new Set(visible.map(row => row.key));
  const wanted = shown ? all ? candidates.map(row => row.tag)
    : mounted.size ? candidates.filter(row => mounted.has(row.key)).map(row => row.tag) : [...new Set([
    ...candidates.filter(row => previousKeys.has(row.key)).map(row => row.tag),
    ...candidates.slice(0, Math.max(visible.length, 1)).map(row => row.tag),
  ])] : [];
  const wantedKey = JSON.stringify([...wanted].sort());
  const cellKey = JSON.stringify([moment, shown?.hour, wantedKey]);
  const cellError = cellFailure?.client === client && cellFailure.owner === owner && cellFailure.key === cellKey
    && cellFailure.attempt === attempt ? cellFailure.error : undefined;
  const supplied = owned?.page?.hour === shown?.hour ? owned?.page : undefined;
  const loaded = new Set(supplied?.data.series.map(row => row.tag));
  const covered = wanted.every(tag => loaded.has(tag)) && !preparedPrefix(supplied)?.pending.length
    && !request?.selection.columns.some(column => fieldPreparing(supplied, column));
  useEffect(() => {
    if (!cellError) setCellFailure(undefined);
    if (!active || !source || !request || !shown || shown.hour !== request.hour || owned?.pending || owned?.error || cellError || covered || !wanted?.length) return;
    const controller = new AbortController();
    const page = scoreboardCellPage({ ...request, hour: shown.hour }, wanted);
    void queryScoreboard(client, source, page, controller.signal, attempt > 0, page => {
      if (!controller.signal.aborted) setState(previous => previous?.client === client && previous.owner === owner && previous.index === shown ? { ...previous, page } : previous);
    }).catch(error => { if (!controller.signal.aborted) setCellFailure({ client, owner, key: cellKey, attempt, error: String(error) }); });
    return () => controller.abort();
  }, [client, owner, shown, cellKey, active, covered, owned?.pending, owned?.error, cellError, attempt]);
  return { index: shown, page: supplied, pending: owned?.pending ?? (active && !!request), error: owned?.error ?? cellError,
    retry: () => setAttempt(value => value + 1), complete: !!shown && covered && shown.data.series.every(row => loaded.has(row.tag)),
    prepare: () => setAll(true) };
}
