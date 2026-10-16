import type { PreparationRequest, PreparationReply } from '../../shell/preparation/contract';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { Artwork, FocusEntry } from '../../generated/dashboard';
import { newId } from '../../shell/ids.ts';
import { artworkEntry, drawableIcon } from '../../dashboard/artwork.ts';

export interface FocusPages { first: string; count: string; rows: string }
export interface PreparedFocusHour {
  schema: number; slot: number; countryId: number; hour: number; lastHour: number; orderFirstHour: number;
  current: string | null; continuous: string | null; paused: boolean;
  progressRaw: string; dailyGainRaw: string | null;
  completed: FocusPages; presentation: FocusPages; details: FocusPages | null; warnings: string[];
}
export interface PreparedFocusArt { artwork: Artwork; reason: string | null }
export interface PreparedFocusSnapshot {
  hour: PreparedFocusHour; order: FocusEntry[]; art: Map<string, PreparedFocusArt>; page: bigint; displayHour: number;
  staticText: Artwork[];
  details?: Map<number, PreparedFocusArt>;
}
export interface FocusPreparedClient extends DashboardClient {
  preparationStatus?(request: PreparationRequest, signal: AbortSignal): Promise<PreparationReply>;
  prepareSource?(request: PreparationRequest, signal: AbortSignal): Promise<PreparationReply>;
  preparedQuery?<T>(source: DashboardSource, product: string, params: unknown, options: { signal: AbortSignal }): Promise<T>;
}
const natural = (value: unknown): value is string => typeof value === 'string' && /^(0|[1-9]\d*)$/.test(value);
const integer = (value: unknown): value is string => typeof value === 'string' && /^-?\d+$/.test(value);
const optionalText = (value: unknown) => value === null || typeof value === 'string';
const pagesValid = (value: FocusPages) => !!value && natural(value.first) && natural(value.count) && natural(value.rows)
  && BigInt(value.count) === (BigInt(value.rows) + 127n) / 128n;
export function validatePreparedHour(value: unknown, hour: number): PreparedFocusHour | null {
  if (value === null) return null;
  const row = value as PreparedFocusHour;
  if (!row || row.schema !== 5 || !Number.isInteger(row.slot) || row.slot < 0 || !Number.isInteger(row.countryId) || row.countryId < 0 || !Number.isInteger(row.hour) || !Number.isInteger(row.lastHour)
    || row.hour > hour || row.lastHour < hour || !Number.isInteger(row.orderFirstHour) || row.orderFirstHour > row.hour || !optionalText(row.current) || !optionalText(row.continuous)
    || typeof row.paused !== 'boolean' || !integer(row.progressRaw)
    || !(row.dailyGainRaw === null || integer(row.dailyGainRaw) && BigInt(row.dailyGainRaw) > 0n)
    || !pagesValid(row.completed) || !pagesValid(row.presentation) || !(row.details === null || pagesValid(row.details))
    || !Array.isArray(row.warnings) || !row.warnings.every(warning => typeof warning === 'string')) throw new Error('Invalid prepared focus hour.');
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
export function validatePreparedArt(value: unknown, pages?: FocusPages, page?: bigint): PreparedFocusArt[] {
  if (!Array.isArray(value) || !value.every(item => item && artworkEntry(item.artwork) && optionalText(item.reason))) throw new Error('Invalid prepared focus artwork.');
  if (pages && page !== undefined) {
    const remaining = BigInt(pages.rows) - page * 128n;
    if (remaining <= 0n || BigInt(value.length) !== (remaining < 128n ? remaining : 128n)) throw new Error('Prepared focus artwork omitted rows.');
  }
  return value.map(item => ({ ...item, artwork: { ...item.artwork, icon: drawableIcon(item.artwork.icon) } }));
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
    if (!started && client.prepareSource && restart && ['idle', 'failed', 'cancelled'].includes(reply.state)) {
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
      if (!changed && Date.now() - before < POLL_MS) throw new Error('Focus preparation made no progress.');
    } else throw new Error(reply.error ?? (reply.state === 'idle' ? (reply.backgroundEnabled ? 'Prepare this recording from Preparation and storage.' : 'Background preparation is off. Prepare this recording or enable it in Settings.') : reply.state === 'cancelled' ? 'Focus preparation cancelled.' : 'Focus preparation failed.'));
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
      if (!changed && Date.now() - before < POLL_MS) throw new Error('Focus preparation made no progress.');
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
  if (!row || row.schema !== 5 || !Number.isInteger(row.firstHour) || !Number.isInteger(row.lastHour)
    || row.firstHour > hour || row.lastHour < hour || !Array.isArray(row.reasons)
    || !row.reasons.every(item => item && typeof item.table === 'string' && optionalText(item.reason)
      && (item.missingRows === null || natural(item.missingRows)) && (item.reason !== null || item.missingRows !== null))) throw new Error('Invalid prepared focus unavailable read.');
  return row.reasons.map(item => item.reason === null ? item.table + ': ' + item.missingRows + ' missing rows' : item.table + ': ' + item.reason);
}

export async function loadPreparedFocusDetail(client: FocusPreparedClient, source: DashboardSource, branchId: string, tag: string,
  displayed: PreparedFocusHour, entry: FocusEntry, signal: AbortSignal): Promise<PreparedFocusArt | null> {
  if (!client.preparedQuery) return null;
  const date = entry.completedByHour ?? displayed.orderFirstHour;
  const row = date >= displayed.hour && date <= displayed.lastHour ? displayed : validatePreparedHour(await client.preparedQuery<unknown>(source,
    'focus-hour', { branchId, tag, hour: date }, { signal }), date);
  if (!row?.details) return null;
  const page = BigInt(entry.ordinal) / 128n;
  const items = validatePreparedArt(await client.preparedQuery<unknown>(source, 'focus-details-page',
    { branchId, tag, hour: date, pages: row.details, page: String(page) }, { signal }), row.details, page);
  if (signal.aborted) throw new DOMException('Focus details cancelled', 'AbortError');
  const item = items[entry.ordinal % 128];
  if (!item || item.artwork.token !== entry.token) throw new Error('Prepared focus details omitted requested token.');
  return item;
}

export const preparedArtKey = (hour: number, token: string) => JSON.stringify([hour, token]);
export async function loadPreparedFocusStaticText(client: FocusPreparedClient, source: DashboardSource, tokens: readonly string[], signal: AbortSignal) {
  const text: Artwork[] = [];
  for (let first = 0; first < tokens.length; first += 128) {
    if (signal.aborted) throw new DOMException('Focus static text cancelled', 'AbortError');
    const wanted = tokens.slice(first, first + 128);
    const values = await client.preparedQuery!<unknown>(source, 'focus-static-text', { tokens: wanted }, { signal });
    if (!Array.isArray(values) || values.length !== wanted.length || !values.every((item, index) => artworkEntry(item) && item.token === wanted[index]))
      throw new Error('Prepared focus static text omitted requested tokens.');
    text.push(...values);
  }
  return text;
}
export async function loadPreparedFocusPage(client: FocusPreparedClient, source: DashboardSource, branchId: string, tag: string,
  hour: number, row: PreparedFocusHour, page: bigint, signal: AbortSignal) {
  if (!client.preparedQuery) throw new Error('Prepared focus queries are unavailable.');
  const query = client.preparedQuery.bind(client);
  const params = { branchId, tag, hour, page: page.toString() };
  const order = BigInt(row.completed.count) === 0n ? [] : validatePreparedOrder(await query<unknown>(source,
    'focus-order-page', { ...params, pages: row.completed }, { signal }), row.completed, page);
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  const wanted = new Map<number, Set<bigint>>();
  const want = (date: number, artPage: bigint) => { const pages = wanted.get(date) ?? new Set<bigint>(); pages.add(artPage); wanted.set(date, pages); };
  for (const entry of order) want(entry.completedByHour ?? row.orderFirstHour, BigInt(entry.ordinal) / 128n);
  const active = row.current ?? row.continuous;
  if (active) want(hour, BigInt(row.completed.rows) / 128n);
  const art = new Map<string, PreparedFocusArt>();
  const presentationReads = new Map<string, Promise<PreparedFocusArt[]>>();
  let staticReason: string | null = null;
  const [staticText] = await Promise.all([loadPreparedFocusStaticText(client, source,
    [...new Set([...order.map(entry => entry.token), ...(active ? [active] : [])])].sort(), signal).catch(error => {
      if (signal.aborted) throw error;
      staticReason = error instanceof Error ? error.message : String(error); return [];
    }), Promise.all([...wanted].map(async ([date, pages]) => {
    try {
    const dated = date >= row.hour && date <= row.lastHour ? row : validatePreparedHour(await query<unknown>(source,
      'focus-hour', { branchId, tag, hour: date }, { signal }), date);
    if (!dated) throw new Error('Prepared focus artwork hour is unavailable.');
    if (dated.presentation.count === '0') throw new Error(dated.warnings.join('; ') || 'Prepared focus presentation is unavailable.');
    await Promise.all([...pages].map(async artPage => {
      const identity = JSON.stringify([dated.slot, dated.countryId, dated.presentation.first, dated.presentation.count, dated.presentation.rows, String(artPage)]);
      let reading = presentationReads.get(identity);
      if (!reading) {
        reading = query<unknown>(source, 'focus-presentation-page',
          { branchId, tag, hour: date, pages: dated.presentation, page: artPage.toString() }, { signal })
          .then(value => validatePreparedArt(value, dated.presentation, artPage));
        presentationReads.set(identity, reading);
      }
      const values = await reading;
      for (const item of values) art.set(preparedArtKey(date, item.artwork.token), item);
    }));
    } catch (error) {
      if (signal.aborted) throw error;
      const reason = error instanceof Error ? error.message : String(error);
      const missing = order.filter(entry => (entry.completedByHour ?? row.orderFirstHour) === date).map(entry => entry.token);
      if (active && date === hour) missing.push(active);
      for (const token of missing) art.set(preparedArtKey(date, token), { artwork: { token, name: null, description: null, baseDurationDays: null, icon: null }, reason });
    }
  }))]);
  if (signal.aborted) throw new DOMException('Focus query cancelled', 'AbortError');
  if (order.some(entry => !art.has(preparedArtKey(entry.completedByHour ?? row.orderFirstHour, entry.token)))
    || active && !art.has(preparedArtKey(hour, active))) throw new Error('Prepared focus artwork omitted requested tokens.');
  const text = new Map(staticText.map(item => [item.token, item]));
  for (const [key, item] of art) art.set(key, { ...item, reason: item.reason ?? staticReason, artwork: { ...item.artwork,
    description: item.artwork.description ?? text.get(item.artwork.token)?.description ?? null } });
  return { order, art, staticText, displayHour: hour };
}
