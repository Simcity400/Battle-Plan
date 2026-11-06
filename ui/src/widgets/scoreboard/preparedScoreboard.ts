import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { fieldPreparing, preparedPrefix } from '../../dashboard/preparedPrefix.ts';
import { useEffect, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries.ts';
import { int, object } from '../../dashboard/validation.ts';
import type { ScoreboardColumnKey, ScoreboardPreparedParams, ScoreboardPreparedReply, ScoreboardSeries } from '../../generated/dashboard';
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
  if (value.since !== undefined && !(int(value.since) && Array.isArray(value.ended) && value.ended.every(tag => typeof tag === 'string')
    && value.ended.every(tag => !data.series.some(series => series.tag === tag)))) throw new Error('Invalid prepared scoreboard rows.');
  return { ...value, hour: value.hour, data, maxima: value.maxima as ScoreboardPreparedReply['maxima'],
    ...(value.since === undefined ? {} : { since: value.since as number, ended: value.ended as string[] }) };
}

/** Whether an index answers every row and cell of its hour for good: what a later hour's rows are read against. */
const completeIndex = (reply: ScoreboardPreparedReply) => !['pending', 'coverage', 'approximate', 'preparing'].some(field => field in reply);

/** A row read at one moment as the same row reads at `hour`: its spans that hour, and a read supplied every hour supplied then. */
function restampedRow(series: ScoreboardSeries, hour: number): ScoreboardSeries {
  const at = <T extends { firstHour: number; lastHour: number }>(span: T): T => ({ ...span, firstHour: hour, lastHour: hour });
  return { ...series, segments: series.segments.map(at), identities: series.identities.map(at), membership: series.membership && series.membership.map(at),
    columns: series.columns.map(column => ({ ...column, runs: column.runs.map(at),
      readRuns: column.readRuns.map(read => ({ ...at(read), readHour: read.hourly ? hour : read.readHour })) })) };
}

/** `reply` joined to `held`, the complete index at the hour it was read against: the index a whole reply at its hour
 * gives. Null when it was read against another hour than the one held. */
export function joinScoreboardHour(held: ScoreboardPreparedReply | undefined, reply: ScoreboardPreparedReply): ScoreboardPreparedReply | null {
  if (reply.since === undefined) return reply;
  if (!held || held.hour !== reply.since || held.data.branchId !== reply.data.branchId) return null;
  const sent = new Set(reply.data.series.map(series => series.tag));
  const ended = new Set(reply.ended ?? []);
  const rows = new Set(held.data.series.map(series => series.tag));
  if ([...ended].some(tag => !rows.has(tag))) return null;
  const series = [...held.data.series.filter(row => !sent.has(row.tag) && !ended.has(row.tag)).map(row => restampedRow(row, reply.hour)), ...reply.data.series];
  const { since: _since, ended: _ended, ...whole } = reply;
  return { ...whole, data: { ...reply.data, series } };
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
  const [state, setState] = useState<{ client: DashboardClient; owner: string; index?: ScoreboardPreparedReply; page?: ScoreboardPreparedReply; pending: boolean; error?: Failure }>();
  const [cellFailure, setCellFailure] = useState<{ client: DashboardClient; owner: string; key: string; attempt: number; error: Failure }>();
  const owned = state?.client === client && state.owner === owner ? state : undefined;
  const heldIndex = useRef<ScoreboardPreparedReply | undefined>(undefined);
  heldIndex.current = owned?.index;
  useEffect(() => {
    if (!active || !source || !request) return;
    const controller = new AbortController();
    // Another hour's rows are read against the complete index held, and joined to it.
    const held = heldIndex.current;
    const base = held && request.index && held.hour !== request.hour && completeIndex(held) ? held : undefined;
    const asked = base ? { ...request, since: base.hour } : request;
    setState(previous => ({ client, owner, index: previous?.client === client && previous.owner === owner ? previous.index : undefined,
      page: previous?.client === client && previous.owner === owner ? previous.page : undefined, pending: true }));
    void queryScoreboard(client, source, asked, controller.signal, attempt > 0, reply => {
      if (controller.signal.aborted) return;
      const index = joinScoreboardHour(base, reply);
      setState(previous => {
        const page = previous?.client === client && previous.owner === owner ? previous.page : undefined;
        return index ? { client, owner, index, page, pending: false }
          : { ...previous, client, owner, pending: false, error: asFailure(new Error('The scoreboard rows were read against another hour.')) };
      });
    }).catch(error => { if (!controller.signal.aborted) setState(previous => ({ ...previous, client, owner, pending: false, error: asFailure(error) })); });
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
    }).catch(error => { if (!controller.signal.aborted) setCellFailure({ client, owner, key: cellKey, attempt, error: asFailure(error) }); });
    return () => controller.abort();
  }, [client, owner, shown, cellKey, active, covered, owned?.pending, owned?.error, cellError, attempt]);
  return { index: shown, page: supplied, pending: owned?.pending ?? (active && !!request), error: owned?.error ?? cellError,
    retry: () => setAttempt(value => value + 1), complete: !!shown && covered && shown.data.series.every(row => loaded.has(row.tag)),
    prepare: () => setAll(true) };
}
