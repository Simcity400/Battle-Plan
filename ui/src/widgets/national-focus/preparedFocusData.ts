import type { PreparationRequest, PreparationReply } from '../../shell/preparation/contract';
import { validateProductReply } from '../../dashboard/client.ts';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { Artwork, FocusEntry } from '../../generated/dashboard';
import { newId } from '../../shell/ids.ts';
import { artworkEntry, drawableIcon } from '../../dashboard/artwork.ts';

export interface FocusPages { sampled?: boolean; first: string; count: string; rows: string }
/** A country's prepared focus order from `hour` through `lastHour`, with its
 * progress and daily gain at `at`, the hour asked. */
export interface PreparedFocusHour {
  schema: number; slot: number; countryId: number; hour: number; lastHour: number; orderFirstHour: number | null; preparing?: string[];
  current: string | null; continuous: string | null; paused: boolean;
  completed: FocusPages; at: number; progressRaw: string; dailyGainRaw: string | null;
}
export interface PreparedFocusArt { artwork: Artwork; reason: string | null }
export interface PreparedFocusSnapshot {
  hour: PreparedFocusHour; order: FocusEntry[]; art: Map<string, PreparedFocusArt>; page: bigint; displayHour: number;
  staticText: Artwork[];
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
export function validatePreparedHour(value: unknown, hour: number): PreparedFocusHour | null {
  if (value === null) return null;
  const row = value as PreparedFocusHour;
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

/** `tokens`' focus art for `tag` at `hour`, evaluated when asked: one request
 * per page of distinct tokens. A request's named failure marks each of its
 * tokens; its warnings are returned once. */
export async function loadFocusArt(client: DashboardClient, source: DashboardSource, branchId: string, tag: string,
  hour: number, tokens: readonly string[], mode: FocusArtMode, signal: AbortSignal) {
  const wanted = [...new Set(tokens)].sort(focusTokenOrder);
  const art = new Map<string, PreparedFocusArt>();
  const warnings = new Set<string>();
  for (let first = 0; first < wanted.length; first += PAGE_ROWS) {
    if (signal.aborted) throw new DOMException('Focus art cancelled', 'AbortError');
    const page = wanted.slice(first, first + PAGE_ROWS);
    const request = { sourceId: source.id, revision: source.revision, product: 'focus-art', params: { branchId, tag, hour, tokens: page, mode } };
    const reply = validateProductReply(await client.product<unknown>(request, signal), request).data as
      { art?: unknown; warnings?: unknown; failures?: unknown } | null;
    if (signal.aborted) throw new DOMException('Focus art cancelled', 'AbortError');
    if (!reply || !Array.isArray(reply.warnings) || !reply.warnings.every(item => typeof item === 'string') || !Array.isArray(reply.failures)
      || !reply.failures.every((item: { reason?: unknown; tokens?: unknown }) => item && typeof item.reason === 'string' && Number.isInteger(item.tokens)))
      throw new Error('Invalid focus art reply.');
    for (const warning of reply.warnings as string[]) warnings.add(warning);
    const failures = reply.failures as { reason: string; tokens: number }[];
    const items = validatePreparedArt(reply.art);
    if (failures.length) {
      const reason = failures.map(failure => failure.reason).join('; ');
      for (const token of page) art.set(token, placeholder(token, reason));
      continue;
    }
    if (items.length !== page.length || items.some((item, index) => item.artwork.token !== page[index])) throw new Error('Focus art omitted requested tokens.');
    for (const item of items) art.set(item.artwork.token, item);
  }
  return { art, warnings: [...warnings] };
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
    } else if (reply.state === 'queued' || reply.state === 'preparing') {
      const next = preparationProgress(reply);
      const changed = next !== progress;
      progress = next;
      const before = Date.now();
      await wait(signal);
      if (signal.aborted) break;
      if (!changed && Date.now() - before < POLL_MS / 2) throw new Error('Focus preparation made no progress.');
    } else throw new Error(reply.error ?? (reply.state === 'paused' ? 'Focus preparation is paused and resumes automatically. Prepare this recording to continue now.' : reply.state === 'idle' ? (reply.backgroundEnabled ? 'Prepare this recording from Preparation and storage.' : 'Background preparation is off. Prepare this recording or enable it in Settings.') : reply.state === 'cancelled' ? 'Focus preparation cancelled.' : 'Focus preparation failed.'));
  }
  throw new DOMException('Focus preparation cancelled', 'AbortError');
}

/** Null is missing coverage only once preparation has finished; an accepted
 * prefix can grow while this particular requested hour is still unprepared. */
export async function loadPreparedFocusHour(client: FocusPreparedClient, source: DashboardSource, branchId: string, tag: string,
  hour: number, signal: AbortSignal, restart = false, wait: (signal: AbortSignal) => Promise<void> = pause): Promise<PreparedFocusHour | null> {
  if (!client.preparedQuery) throw new Error('Prepared focus queries are unavailable.');
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  const ready = validatePreparedHour(await client.preparedQuery<unknown>(source, 'focus-hour', { branchId, tag, hour }, { signal }), hour);
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  if (ready || !client.preparationStatus) return ready;
  await waitForFocusPreparation(client, source, signal, restart, wait);
  let progress: string | undefined;
  while (!signal.aborted) {
    const row = validatePreparedHour(await client.preparedQuery<unknown>(source, 'focus-hour', { branchId, tag, hour }, { signal }), hour);
    if (signal.aborted) break;
    if (row || !client.preparationStatus) return row;
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
export function validateUnavailableFocus(value: unknown, hour: number): string[] {
  if (value === null) return [];
  const row = value as FocusUnavailableHour;
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
  const { art } = await loadFocusArt(client, source, branchId, tag, date, [entry.token], 'text', signal);
  if (signal.aborted) throw new DOMException('Focus details cancelled', 'AbortError');
  return art.get(entry.token) ?? null;
}

export const preparedArtKey = (hour: number, token: string) => JSON.stringify([hour, token]);
/** The tokens' static text, and the content warnings its replies name once. */
export async function loadPreparedFocusStaticText(client: FocusPreparedClient, source: DashboardSource, tokens: readonly string[], signal: AbortSignal) {
  const text: Artwork[] = [];
  const warnings = new Set<string>();
  for (let first = 0; first < tokens.length; first += 128) {
    if (signal.aborted) throw new DOMException('Focus static text cancelled', 'AbortError');
    const wanted = tokens.slice(first, first + 128);
    const reply = await client.preparedQuery!<unknown>(source, 'focus-static-text', { tokens: wanted }, { signal }) as { texts?: unknown; warnings?: unknown } | null;
    const values = reply?.texts;
    if (!reply || !Array.isArray(reply.warnings) || !reply.warnings.every(item => typeof item === 'string')
      || !Array.isArray(values) || values.length !== wanted.length || !values.every((item, index) => artworkEntry(item) && item.token === wanted[index]))
      throw new Error('Prepared focus static text omitted requested tokens.');
    text.push(...values);
    for (const warning of reply.warnings as string[]) warnings.add(warning);
  }
  return { text, warnings: [...warnings] };
}
/** One page of completed focuses, each with its art as evaluated at the hour
 * that confirmed it, and the active focus's art at `hour`. */
export async function loadPreparedFocusPage(client: FocusPreparedClient, source: DashboardSource, branchId: string, tag: string,
  hour: number, row: PreparedFocusHour, page: bigint, signal: AbortSignal) {
  if (!client.preparedQuery) throw new Error('Prepared focus queries are unavailable.');
  const query = client.preparedQuery.bind(client);
  const params = { branchId, tag, hour, page: page.toString() };
  const order = BigInt(row.completed.count) === 0n ? [] : validatePreparedOrder(await query<unknown>(source,
    'focus-order-page', { ...params, pages: row.completed }, { signal }), row.completed, page);
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  const wanted = new Map<number, Set<string>>();
  const want = (date: number, token: string) => { const tokens = wanted.get(date) ?? new Set<string>(); tokens.add(token); wanted.set(date, tokens); };
  for (const entry of order) want(entry.completedByHour ?? row.orderFirstHour ?? row.at, entry.token);
  const active = row.current ?? row.continuous;
  if (active) want(hour, active);
  const art = new Map<string, PreparedFocusArt>();
  const warnings = new Set<string>();
  let staticReason: string | null = null;
  const [staticText] = await Promise.all([loadPreparedFocusStaticText(client, source,
    [...new Set([...order.map(entry => entry.token), ...(active ? [active] : [])])].sort(), signal).catch(error => {
      if (signal.aborted) throw error;
      staticReason = error instanceof Error ? error.message : String(error); return { text: [] as Artwork[], warnings: [] as string[] };
    }), Promise.all([...wanted].map(async ([date, tokens]) => {
    try {
      const dated = await loadFocusArt(client, source, branchId, tag, date, [...tokens], 'display', signal);
      for (const warning of dated.warnings) warnings.add(warning);
      for (const [token, item] of dated.art) art.set(preparedArtKey(date, token), item);
    } catch (error) {
      if (signal.aborted) throw error;
      const reason = error instanceof Error ? error.message : String(error);
      for (const token of tokens) art.set(preparedArtKey(date, token), placeholder(token, reason));
    }
  }))]);
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  if (order.some(entry => !art.has(preparedArtKey(entry.completedByHour ?? row.orderFirstHour ?? row.at, entry.token)))
    || active && !art.has(preparedArtKey(hour, active))) throw new Error('Prepared focus artwork omitted requested tokens.');
  // Static descriptions can stand in for refused art, so their content's warnings are the page's.
  for (const warning of staticText.warnings) warnings.add(warning);
  const text = new Map(staticText.text.map(item => [item.token, item]));
  for (const [key, item] of art) art.set(key, { ...item, reason: item.reason ?? staticReason, artwork: { ...item.artwork,
    description: item.artwork.description ?? text.get(item.artwork.token)?.description ?? null } });
  return { order, art, staticText: staticText.text, displayHour: hour, warnings: [...warnings] };
}
