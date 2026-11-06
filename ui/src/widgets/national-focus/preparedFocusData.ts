import { failureText } from '../../dashboard/queryError.ts';
import { asFailure, queryFailure, type Failure } from '../../dashboard/queryError.ts';
import type { PreparationRequest, PreparationReply } from '../../shell/preparation/contract';
import { validateProductReply } from '../../dashboard/client.ts';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { Artwork, FocusArtGroup, FocusDatabase, FocusEntry, QueryError } from '../../generated/dashboard';
import { newId } from '../../shell/ids.ts';
import { artworkEntry, drawableIcon } from '../../dashboard/artwork.ts';
import { activeFocus } from './focusData.ts';

export interface FocusPages { sampled?: boolean; first: string; count: string; rows: string }
/** A country's prepared focus order from `hour` through `lastHour`, with its
 * progress and daily gain at `at`, the hour asked. */
export interface PreparedFocusHour {
  schema: number; slot: number; countryId: number; hour: number; lastHour: number; orderFirstHour: number | null; preparing?: string[];
  current: string | null; continuous: string | null; paused: boolean;
  completed: FocusPages; at: number; progressRaw: string; dailyGainRaw: string | null;
}
export interface PreparedFocusArt { artwork: Artwork; reason: string | null; failure?: Failure; titleFailure?: Failure }
export interface PreparedFocusSnapshot {
  hour: PreparedFocusHour; order: FocusEntry[]; page: bigint; displayHour: number;
  /** By `preparedArtKey`: completions are national, the active focus is in its own database. */
  art: Map<string, PreparedFocusArt>;
  /** The national focuses' static text. */
  staticText: Artwork[];
  /** The continuous focus's static text. */
  continuousStaticText: Artwork[];
  /** The content's standing warnings and the art requests' named failures. */
  warnings: string[];
  details?: Map<number, PreparedFocusArt>;
}
export interface FocusPreparedClient extends DashboardClient {
  preparationStatus?(request: PreparationRequest, signal: AbortSignal): Promise<PreparationReply>;
  prepareSource?(request: PreparationRequest, signal: AbortSignal): Promise<PreparationReply>;
  preparedQuery?<T>(source: DashboardSource, product: string, params: unknown, options: { signal: AbortSignal }): Promise<T>;
}
const PREPARED_FOCUS_SCHEMA = 6;
const PAGE_ROWS = 128;
const natural = (value: unknown): value is string => typeof value === 'string' && /^(0|[1-9]\d*)$/.test(value);
const integer = (value: unknown): value is string => typeof value === 'string' && /^-?\d+$/.test(value);
const optionalText = (value: unknown) => value === null || typeof value === 'string';
const pagesValid = (value: FocusPages) => !!value && natural(value.first) && natural(value.count) && natural(value.rows)
  && (value.sampled === undefined || typeof value.sampled === 'boolean')
  && BigInt(value.count) === (BigInt(value.rows) + 127n) / 128n;
/** A `focus-hour` reply's focus: null for a pending (null) reply, and where the country was not observed. */
export function validatePreparedHour(value: unknown, hour: number): PreparedFocusHour | null {
  if (value === null) return null;
  const reply = value as { focus?: unknown } | undefined;
  if (!reply || typeof reply !== 'object' || reply.focus === undefined) throw new Error('Invalid prepared focus hour.');
  if (reply.focus === null) return null;
  const row = reply.focus as PreparedFocusHour;
  if (!row || row.schema !== PREPARED_FOCUS_SCHEMA || !Number.isInteger(row.slot) || row.slot < 0 || !Number.isInteger(row.countryId) || row.countryId < 0 || !Number.isInteger(row.hour) || !Number.isInteger(row.lastHour)
    || row.hour > hour || row.lastHour < hour || row.at !== hour || !(row.orderFirstHour === null && row.preparing?.includes('orderFirstHour') || Number.isInteger(row.orderFirstHour) && row.orderFirstHour! <= row.hour) || !optionalText(row.current) || !optionalText(row.continuous)
    || row.preparing !== undefined && (!Array.isArray(row.preparing) || !row.preparing.every(field => typeof field === 'string'))
    || typeof row.paused !== 'boolean' || !integer(row.progressRaw)
    || !(row.dailyGainRaw === null || integer(row.dailyGainRaw) && BigInt(row.dailyGainRaw) > 0n)
    || !pagesValid(row.completed)) throw new Error('Invalid prepared focus hour.');
  return row;
}
export const lastPage = (pages: FocusPages) => BigInt(pages.count) > 0n ? BigInt(pages.count) - 1n : 0n;
export function validatePreparedOrder(value: unknown, pages: FocusPages, page: bigint): FocusEntry[] {
  if (!Array.isArray(value)) throw new Error('Prepared focus order page is unavailable.');
  const first = page * 128n;
  const expected = BigInt(pages.rows) - first;
  if (page < 0n || page >= BigInt(pages.count) || BigInt(value.length) !== (expected < 128n ? expected : 128n)) throw new Error('Prepared focus order page omitted rows.');
  if (!value.every((entry: FocusEntry, index) => entry && Number.isInteger(entry.ordinal)
    && BigInt(entry.ordinal) === first + BigInt(index) && typeof entry.token === 'string' && entry.token.length > 0
    && (entry.completedByHour === null || Number.isInteger(entry.completedByHour)))) throw new Error('Invalid prepared focus order.');
  return value;
}
export function validatePreparedArt(value: unknown): PreparedFocusArt[] {
  if (!Array.isArray(value) || !value.every(item => item && artworkEntry(item.artwork) && optionalText(item.reason))) throw new Error('Invalid prepared focus artwork.');
  return value.map(item => ({ ...item, artwork: { ...item.artwork, icon: drawableIcon(item.artwork.icon) } }));
}

/** Tokens in the service's order: by Unicode scalar value, as UTF-8 bytes compare. */
export function focusTokenOrder(a: string, b: string): number {
  const left = [...a], right = [...b];
  for (let index = 0; index < Math.min(left.length, right.length); index++) {
    const difference = left[index].codePointAt(0)! - right[index].codePointAt(0)!;
    if (difference) return difference;
  }
  return left.length - right.length;
}

export type FocusArtMode = 'display' | 'text';
const placeholder = (token: string, reason: string): PreparedFocusArt =>
  ({ artwork: { token, name: null, description: null, baseDurationDays: null, icon: null }, reason });

/** Tokens wanted at an hour, each looked up in `database`. */
export interface FocusArtAsk { hour: number; database: FocusDatabase; tokens: Iterable<string> }
/** What `loadFocusArt` read, each by `preparedArtKey`. */
export interface LoadedFocusArt {
  /** Each token's art as evaluated. */
  art: Map<string, PreparedFocusArt>;
  /** Placeholders naming why an hour's evaluation failed. */
  failed: Map<string, PreparedFocusArt>;
  /** Tokens of an hour the service refused or failed alone. */
  errors: Map<string, { token: string; failure: Failure }>;
  warnings: string[];
}
/** Most tokens one request asks for: the service's limit, a page and the active focus. */
const ART_TOKENS = PAGE_ROWS + 1;
/** The service's order of databases. */
const DATABASE_ORDER: Record<FocusDatabase, number> = { national: 0, continuous: 1 };
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
const queryError = (value: unknown): value is QueryError => !!value && typeof value === 'object'
  && ((value as { kind?: unknown }).kind === 'refused' && typeof (value as { text?: unknown }).text === 'string'
    || (value as { kind?: unknown }).kind === 'failed' && typeof (value as { message?: unknown }).message === 'string');

/** The asked tokens' focus art for `tag`, evaluated when asked: every hour of a page in one request,
 * each token at its hour, looked up in its database. An hour's named failure marks each of its
 * tokens; one the service refused or failed alone is that hour's error; warnings are returned once. */
export async function loadFocusArt(client: DashboardClient, source: DashboardSource, branchId: string, tag: string,
  asks: readonly FocusArtAsk[], mode: FocusArtMode, signal: AbortSignal): Promise<LoadedFocusArt> {
  const merged = new Map<string, { hour: number; database: FocusDatabase; tokens: Set<string> }>();
  for (const ask of asks) {
    const key = `${ask.hour}:${ask.database}`;
    let group = merged.get(key);
    if (!group) merged.set(key, group = { hour: ask.hour, database: ask.database, tokens: new Set() });
    for (const token of ask.tokens) group.tokens.add(token);
  }
  const groups = [...merged.values()].map(({ hour, database, tokens }) => ({ hour, database, tokens: [...tokens].sort(focusTokenOrder) }))
    .sort((a, b) => a.hour - b.hour || DATABASE_ORDER[a.database] - DATABASE_ORDER[b.database]);
  // Requests within the service's limit, an hour split across two only where one would pass it.
  const requests: FocusArtGroup[][] = [];
  let batch: FocusArtGroup[] = [], size = 0;
  for (const group of groups) {
    for (let first = 0; first < group.tokens.length;) {
      const tokens = group.tokens.slice(first, first + ART_TOKENS - size);
      batch.push({ hour: group.hour, database: group.database, tokens });
      first += tokens.length; size += tokens.length;
      if (size === ART_TOKENS) { requests.push(batch); batch = []; size = 0; }
    }
  }
  if (batch.length) requests.push(batch);
  const loaded: LoadedFocusArt = { art: new Map(), failed: new Map(), errors: new Map(), warnings: [] };
  const warnings = new Set<string>();
  for (const asked of requests) {
    if (signal.aborted) throw new DOMException('Focus art cancelled', 'AbortError');
    const request = { sourceId: source.id, revision: source.revision, product: 'focus-art', params: { branchId, tag, groups: asked, mode } };
    const reply = validateProductReply(await client.product<unknown>(request, signal), request).data as
      { groups?: unknown; warnings?: unknown } | null;
    if (signal.aborted) throw new DOMException('Focus art cancelled', 'AbortError');
    if (!reply || !strings(reply.warnings) || !Array.isArray(reply.groups) || reply.groups.length !== asked.length) throw new Error('Invalid focus art reply.');
    for (const warning of reply.warnings) warnings.add(warning);
    asked.forEach((group, index) => {
      const answer = (reply.groups as unknown[])[index] as { art?: unknown; failures?: unknown; error?: unknown } | null;
      if (!answer || typeof answer !== 'object' || !(answer.error === null || queryError(answer.error)) || !Array.isArray(answer.failures)
        || !answer.failures.every((item: { reason?: unknown; tokens?: unknown }) => item && typeof item.reason === 'string' && Number.isInteger(item.tokens)))
        throw new Error('Invalid focus art reply.');
      const key = (token: string) => preparedArtKey(group.database, group.hour, token);
      if (answer.error) {
        const failure = queryFailure(answer.error as QueryError);
        for (const token of group.tokens) loaded.errors.set(key(token), { token, failure });
        return;
      }
      const failures = answer.failures as { reason: string; tokens: number }[];
      const items = validatePreparedArt(answer.art);
      if (failures.length) {
        const reason = failures.map(failure => failure.reason).join('; ');
        for (const token of group.tokens) loaded.failed.set(key(token), placeholder(token, reason));
        return;
      }
      if (items.length !== group.tokens.length || items.some((item, at) => item.artwork.token !== group.tokens[at])) throw new Error('Focus art omitted requested tokens.');
      for (const item of items) loaded.art.set(key(item.artwork.token), item);
    });
  }
  loaded.warnings = [...warnings];
  return loaded;
}

const POLL_MS = 500;
const pause = (signal: AbortSignal) => new Promise<void>((resolve, reject) => {
  if (signal.aborted) { reject(new DOMException('Focus preparation cancelled', 'AbortError')); return; }
  const aborted = () => { clearTimeout(timer); reject(new DOMException('Focus preparation cancelled', 'AbortError')); };
  const timer = setTimeout(() => { signal.removeEventListener('abort', aborted); resolve(); }, POLL_MS);
  signal.addEventListener('abort', aborted, { once: true });
});
const preparationProgress = (reply: PreparationReply) => JSON.stringify([reply.state,
  reply.sections.map(section => [section.id, section.state, section.done, section.total])]);

/** Paced polling survives unchanged status; an instant stationary wait cannot spin. */
export async function waitForFocusPreparation(client: FocusPreparedClient, source: DashboardSource, signal: AbortSignal,
  restart = false, wait: (signal: AbortSignal) => Promise<void> = pause): Promise<void> {
  if (!client.preparationStatus) return;
  let started = false, progress: string | undefined;
  while (!signal.aborted) {
    const request = { sourceId: source.id, revision: source.revision, requestId: newId() };
    const reply = await client.preparationStatus(request, signal);
    if (signal.aborted) break;
    if (reply.sourceId !== request.sourceId || reply.revision !== request.revision || reply.requestId !== request.requestId) throw new Error('Preparation reply does not match this recording.');
    if (reply.state === 'ready' || reply.sections.some(section => section.id === 'focus' && section.state === 'ready')) return;
    if (!started && client.prepareSource && restart && ['idle', 'failed', 'cancelled', 'paused'].includes(reply.state)) {
      started = true;
      await client.prepareSource({ sourceId: source.id, revision: source.revision, requestId: newId() }, signal);
      progress = undefined;
    } else if (reply.state === 'queued' || reply.state === 'preparing' || reply.state === 'paused') {
      const next = preparationProgress(reply);
      const changed = next !== progress;
      progress = next;
      const before = Date.now();
      await wait(signal);
      if (signal.aborted) break;
      if (!changed && Date.now() - before < POLL_MS / 2) throw new Error('Focus preparation made no progress.');
    } else throw new Error(reply.error ?? (reply.state === 'idle' ? (reply.backgroundEnabled ? 'Prepare this recording from Preparation and storage.' : 'Background preparation is off. Prepare this recording or enable it in Settings.') : reply.state === 'cancelled' ? 'Focus preparation cancelled.' : 'Focus preparation failed.'));
  }
  throw new DOMException('Focus preparation cancelled', 'AbortError');
}

/** A null reply is an hour still preparing, and missing coverage once preparation has finished; an
 * accepted prefix can grow while this particular requested hour is still unprepared. An answered
 * hour names no focus where the country was not observed. */
export async function loadPreparedFocusHour(client: FocusPreparedClient, source: DashboardSource, branchId: string, tag: string,
  hour: number, signal: AbortSignal, restart = false, wait: (signal: AbortSignal) => Promise<void> = pause): Promise<PreparedFocusHour | null> {
  if (!client.preparedQuery) throw new Error('Prepared focus queries are unavailable.');
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  const read = () => client.preparedQuery!<unknown>(source, 'focus-hour', { branchId, tag, hour }, { signal });
  const ready = await read();
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  if (ready !== null || !client.preparationStatus) return validatePreparedHour(ready, hour);
  await waitForFocusPreparation(client, source, signal, restart, wait);
  let progress: string | undefined;
  while (!signal.aborted) {
    const reply = await read();
    if (signal.aborted) break;
    if (reply !== null || !client.preparationStatus) return validatePreparedHour(reply, hour);
    const request = { sourceId: source.id, revision: source.revision, requestId: newId() };
    const status = await client.preparationStatus(request, signal);
    if (signal.aborted) break;
    if (status.sourceId !== request.sourceId || status.revision !== request.revision || status.requestId !== request.requestId) throw new Error('Preparation reply does not match this recording.');
    if (status.state === 'ready') return null;
    if (status.state === 'queued' || status.state === 'preparing') {
      const next = preparationProgress(status);
      const changed = next !== progress;
      progress = next;
      const before = Date.now();
      await wait(signal);
      if (signal.aborted) break;
      if (!changed && Date.now() - before < POLL_MS / 2) throw new Error('Focus preparation made no progress.');
      continue;
    }
    throw new Error(status.error ?? (status.state === 'cancelled' ? 'Focus preparation cancelled.' : 'Focus preparation failed.'));
  }
  throw new DOMException('Focus query cancelled', 'AbortError');
}

/** A loading target cannot inherit a holder merely because its tag is unchanged. */
export function canCarryPreparedFocus(previous: PreparedFocusHour, target: number | PreparedFocusHour): boolean {
  return typeof target === 'number' ? target >= previous.hour && target <= previous.lastHour
    : previous.slot === target.slot && previous.countryId === target.countryId;
}

export interface FocusUnavailableHour {
  schema: number; firstHour: number; lastHour: number;
  reasons: { table: string; reason: string | null; missingRows: string | null }[];
}
/** A `focus-unavailable` reply's refusals: none for a pending (null) reply, and where none refused. */
export function validateUnavailableFocus(value: unknown, hour: number): string[] {
  if (value === null) return [];
  const reply = value as { refused?: unknown } | undefined;
  if (!reply || typeof reply !== 'object' || reply.refused === undefined) throw new Error('Invalid prepared focus unavailable read.');
  if (reply.refused === null) return [];
  const row = reply.refused as FocusUnavailableHour;
  if (!row || row.schema !== PREPARED_FOCUS_SCHEMA || !Number.isInteger(row.firstHour) || !Number.isInteger(row.lastHour)
    || row.firstHour > hour || row.lastHour < hour || !Array.isArray(row.reasons)
    || !row.reasons.every(item => item && typeof item.table === 'string' && optionalText(item.reason)
      && (item.missingRows === null || natural(item.missingRows)) && (item.reason !== null || item.missingRows !== null))) throw new Error('Invalid prepared focus unavailable read.');
  return row.reasons.map(item => item.reason === null ? item.table + ': ' + item.missingRows + ' missing rows' : item.table + ': ' + item.reason);
}

/** A completed focus's description as evaluated at its confirming hour. */
export async function loadPreparedFocusDetail(client: FocusPreparedClient, source: DashboardSource, branchId: string, tag: string,
  displayed: PreparedFocusHour, entry: FocusEntry, signal: AbortSignal): Promise<PreparedFocusArt | null> {
  const date = entry.completedByHour ?? displayed.orderFirstHour ?? displayed.at;
  const loaded = await loadFocusArt(client, source, branchId, tag, [{ hour: date, database: 'national', tokens: [entry.token] }], 'text', signal);
  if (signal.aborted) throw new DOMException('Focus details cancelled', 'AbortError');
  const key = preparedArtKey('national', date, entry.token), error = loaded.errors.get(key);
  if (error) throw error.failure;
  return loaded.art.get(key) ?? loaded.failed.get(key) ?? null;
}

/** One focus's art at an hour: a national and a continuous focus of one id never share it. */
export const preparedArtKey = (database: FocusDatabase, hour: number, token: string) => JSON.stringify([database, hour, token]);
/** The tokens' static text, each looked up in `database`, and the content warnings its replies name once. */
export async function loadPreparedFocusStaticText(client: FocusPreparedClient, source: DashboardSource, database: FocusDatabase,
  tokens: readonly string[], signal: AbortSignal) {
  const text: Artwork[] = [];
  const warnings = new Set<string>();
  for (let first = 0; first < tokens.length; first += 128) {
    if (signal.aborted) throw new DOMException('Focus static text cancelled', 'AbortError');
    const wanted = tokens.slice(first, first + 128);
    const reply = await client.preparedQuery!<unknown>(source, 'focus-static-text', { database, tokens: wanted }, { signal }) as { texts?: unknown; warnings?: unknown } | null;
    const values = reply?.texts;
    if (!reply || !Array.isArray(reply.warnings) || !reply.warnings.every(item => typeof item === 'string')
      || !Array.isArray(values) || values.length !== wanted.length || !values.every((item, index) => artworkEntry(item) && item.token === wanted[index]))
      throw new Error('Prepared focus static text omitted requested tokens.');
    text.push(...values);
    for (const warning of reply.warnings as string[]) warnings.add(warning);
  }
  return { text, warnings: [...warnings] };
}
/** What a focus page read that holds for any hour of the same country, recording and content: art as
 * evaluated (by `preparedArtKey`) and static text (by `staticTextKey`), each with its reply's warnings.
 * Failures are never kept. */
export interface FocusPageReads {
  art: Map<string, { item: PreparedFocusArt; warnings: readonly string[] }>;
  text: Map<string, { item: Artwork; warnings: readonly string[] }>;
}
const staticTextKey = (database: FocusDatabase, token: string) => JSON.stringify([database, token]);

/** One page of completed focuses, each with its art as evaluated at the hour that confirmed it, and
 * the active focus's art at `hour`. What `held` has is not read again; `reads` is what this page used,
 * for the next. */
export async function loadPreparedFocusPage(client: FocusPreparedClient, source: DashboardSource, branchId: string, tag: string,
  hour: number, row: PreparedFocusHour, page: bigint, signal: AbortSignal, held?: FocusPageReads) {
  if (!client.preparedQuery) throw new Error('Prepared focus queries are unavailable.');
  const query = client.preparedQuery.bind(client);
  const params = { branchId, tag, hour, page: page.toString() };
  const order = BigInt(row.completed.count) === 0n ? [] : validatePreparedOrder(await query<unknown>(source,
    'focus-order-page', { ...params, pages: row.completed }, { signal }), row.completed, page);
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  // Completions are national; the active focus is national or continuous, each looked up in its own database.
  const wanted = new Map<string, { database: FocusDatabase; date: number; tokens: Set<string> }>();
  const want = (database: FocusDatabase, date: number, token: string) => {
    const key = JSON.stringify([database, date]), group = wanted.get(key) ?? { database, date, tokens: new Set<string>() };
    group.tokens.add(token); wanted.set(key, group);
  };
  for (const entry of order) want('national', entry.completedByHour ?? row.orderFirstHour ?? row.at, entry.token);
  const active = activeFocus(row);
  if (active?.token) want(active.database, hour, active.token);
  const reads: FocusPageReads = { art: new Map(), text: new Map() };
  const art = new Map<string, PreparedFocusArt>();
  const warnings = new Set<string>();
  const asks: FocusArtAsk[] = [];
  for (const { database, date, tokens } of wanted.values()) {
    const missing: string[] = [];
    for (const token of tokens) {
      const key = preparedArtKey(database, date, token), kept = held?.art.get(key);
      if (!kept) { missing.push(token); continue; }
      reads.art.set(key, kept);
      art.set(key, kept.item);
      for (const warning of kept.warnings) warnings.add(warning);
    }
    if (missing.length) asks.push({ hour: date, database, tokens: missing });
  }
  const loadArt = async () => {
    if (!asks.length) return;
    try {
      const loaded = await loadFocusArt(client, source, branchId, tag, asks, 'display', signal);
      for (const warning of loaded.warnings) warnings.add(warning);
      for (const [key, item] of loaded.art) { art.set(key, item); reads.art.set(key, { item, warnings: loaded.warnings }); }
      for (const [key, item] of loaded.failed) art.set(key, item);
      for (const [key, { token, failure }] of loaded.errors)
        art.set(key, { ...placeholder(token, failureText(failure, 'Couldn’t load focus artwork.')), failure, titleFailure: failure });
    } catch (error) {
      if (signal.aborted) throw error;
      const failure = asFailure(error), reason = failureText(failure, 'Couldn’t load focus artwork.');
      for (const ask of asks) for (const token of ask.tokens)
        art.set(preparedArtKey(ask.database, ask.hour, token), { ...placeholder(token, reason), failure, titleFailure: failure });
    }
  };
  const staticReasons = new Map<FocusDatabase, Failure>();
  const staticTokens = (database: FocusDatabase) => [...new Set([...(database === 'national' ? order.map(entry => entry.token) : []),
    ...(active?.token && active.database === database ? [active.token] : [])])].sort();
  const loadStatic = async (database: FocusDatabase) => {
    const tokens = staticTokens(database), texts = new Map<string, Artwork>(), told = new Set<string>(), missing: string[] = [];
    for (const token of tokens) {
      const keptText = held?.text.get(staticTextKey(database, token));
      if (!keptText) { missing.push(token); continue; }
      reads.text.set(staticTextKey(database, token), keptText);
      texts.set(token, keptText.item);
      for (const warning of keptText.warnings) told.add(warning);
    }
    try {
      const loaded = await loadPreparedFocusStaticText(client, source, database, missing, signal);
      for (const item of loaded.text) {
        texts.set(item.token, item);
        reads.text.set(staticTextKey(database, item.token), { item, warnings: loaded.warnings });
      }
      for (const warning of loaded.warnings) told.add(warning);
      return { text: tokens.flatMap(token => texts.get(token) ?? []), texts, warnings: [...told] };
    } catch (error) {
      if (signal.aborted) throw error;
      // A refused read leaves the database without static text, as the whole page reads it.
      staticReasons.set(database, asFailure(error));
      return { text: [] as Artwork[], texts: new Map<string, Artwork>(), warnings: [] as string[] };
    }
  };
  const [national, continuous] = await Promise.all([loadStatic('national'), loadStatic('continuous'), loadArt()]);
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  if (order.some(entry => !art.has(preparedArtKey('national', entry.completedByHour ?? row.orderFirstHour ?? row.at, entry.token)))
    || active?.token && !art.has(preparedArtKey(active.database, hour, active.token))) throw new Error('Prepared focus artwork omitted requested tokens.');
  // Static descriptions can stand in for refused art, so their content's warnings are the page's.
  for (const warning of [...national.warnings, ...continuous.warnings]) warnings.add(warning);
  const texts = { national: national.texts, continuous: continuous.texts };
  for (const { database, date, tokens } of wanted.values()) for (const token of tokens) {
    const key = preparedArtKey(database, date, token), item = art.get(key)!;
    art.set(key, { ...item, failure: item.failure ?? staticReasons.get(database), reason: item.reason ?? (staticReasons.has(database) ? failureText(staticReasons.get(database), 'Couldn’t load focus text.') : null), artwork: { ...item.artwork,
      description: item.artwork.description ?? texts[database].get(item.artwork.token)?.description ?? null } });
  }
  return { order, art, staticText: national.text, continuousStaticText: continuous.text, displayHour: hour, warnings: [...warnings], reads };
}
