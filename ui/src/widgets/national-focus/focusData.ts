import type { CountryParams, FocusArtworkParams, FocusArtworkReply, FocusDatabase, FocusHistory, FocusMissing, FocusOrder, FocusProgress, FocusReply, FocusSegment } from '../../generated/dashboard';
import type { ProductRequest } from '../../dashboard/client';
import type { NameAt } from '../../dashboard/countryFacts';
import { artworkEntry, drawableIcon, keepDrawableIcons } from '../../dashboard/artwork.ts';
import { enTwoDecimals } from '../../charts/numberFormat.ts';
import { productRequest } from '../../dashboard/request.ts';

export const PRODUCT = 'national-focus';
const FIXED = 100_000n;

export function focusRequest(sourceId: string, revision: string, params: CountryParams): ProductRequest<typeof PRODUCT> {
  return productRequest({ sourceId, revision, product: PRODUCT, params });
}

function spanAt<T extends { firstHour: number; lastHour: number }>(spans: readonly T[], hour: number): number {
  let low = 0, high = spans.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (spans[middle].firstHour <= hour) low = middle + 1;
    else high = middle;
  }
  return spans[low - 1] && hour <= spans[low - 1].lastHour ? low - 1 : -1;
}

export function focusSegmentAt(history: FocusHistory, hour: number): FocusSegment | null {
  const index = spanAt(history.segments, hour);
  return index < 0 ? null : history.segments[index];
}

export function focusAt(history: FocusHistory, hour: number): FocusOrder | null {
  return focusSegmentAt(history, hour)?.order ?? null;
}

export function focusMissingAt(reply: Pick<FocusReply, 'missing'>, hour: number): FocusMissing | null {
  const index = spanAt(reply.missing, hour);
  return index < 0 ? null : reply.missing[index];
}

export function focusShownAt(reply: FocusReply, hour: number | null): string {
  if (hour === null) return 'out';
  const segment = spanAt(reply.history.segments, hour);
  if (segment >= 0) return `s${segment}`;
  const missing = spanAt(reply.missing, hour);
  return missing >= 0 ? `m${missing}` : 'none';
}

export interface SegmentHours { completed: readonly number[]; active: number }

export function focusReadHours(history: FocusHistory): SegmentHours[] {
  const out: SegmentHours[] = [];
  history.segments.forEach((segment, index) => {
    const previous = index > 0 && history.segments[index - 1].lastHour + 1 === segment.firstHour ? history.segments[index - 1] : null;
    const before = previous ? out[index - 1] : null;
    const now = activeFocus(segment.order), then = previous ? activeFocus(previous.order) : null;
    const active = before && now && then && now.database === then.database && now.token === then.token ? before.active : segment.firstHour;
    const completed = segment.order.completed.map((entry, ordinal) => {
      if (entry.completedByHour !== null) return entry.completedByHour;
      const held = previous?.order.completed[ordinal];
      return before && held && held.token === entry.token && held.completedByHour === null ? before.completed[ordinal] : segment.firstHour;
    });
    out.push({ completed, active });
  });
  return out;
}

export function focusArtAsks(order: FocusOrder | null, hours: SegmentHours | undefined, live: boolean, moment: boolean): NameAt[] {
  if (!order || hours === undefined) return [];
  const rows = order.completed.map((entry, ordinal) => ({ tag: entry.token, hour: hours.completed[ordinal] }));
  if (live) rows.reverse();
  const current = activeFocus(order);
  if (current?.database === 'national') {
    const item = { tag: current.token, hour: hours.active };
    if (moment) rows.unshift(item); else rows.push(item);
  }
  return rows.filter(row => Number.isInteger(row.hour));
}

export function focusAsksAhead(asks: readonly NameAt[], segments: readonly FocusSegment[], hours: readonly SegmentHours[], segment: number, until: number,
  asksOf: (order: FocusOrder, hours: SegmentHours) => NameAt[]): NameAt[] {
  if (segment < 0 || segment + 1 >= segments.length || segments[segment + 1].firstHour >= until) return [...asks];
  const out = [...asks], seen = new Set(asks.map(ask => `${ask.hour}\u0000${ask.tag}`));
  for (let next = segment + 1; next < segments.length && segments[next].firstHour < until; next++) {
    for (const ask of asksOf(segments[next].order, hours[next])) {
      const key = `${ask.hour}\u0000${ask.tag}`;
      if (!seen.has(key)) { seen.add(key); out.push(ask); }
    }
  }
  return out;
}

export function focusAheadUntil(nextHour: number, hoursPerSecond: number, aheadSeconds: number): number {
  const stretch = 2 * Math.max(1, Math.ceil(hoursPerSecond * aheadSeconds));
  return (Math.floor(nextHour / stretch) + 1) * stretch + stretch / 2;
}

export function continuousArtAsks(order: FocusOrder | null, hours: SegmentHours | undefined): NameAt[] {
  const current = activeFocus(order);
  return current?.database === 'continuous' && hours !== undefined ? [{ tag: current.token, hour: hours.active }] : [];
}

export const focusDays = (value: number) => `${enTwoDecimals.format(value)} ${value === 1 ? 'day' : 'days'}`;

export interface FocusProgressView {
  days: string | null;
  left: { value: string; unit: string; estimated: boolean } | null;
  fraction: number | null;
  banked: string | null;
}

export function focusProgressView(state: FocusProgressState | null, baseDurationDays: number | null | undefined, current: boolean): FocusProgressView {
  if (!current) {
    const banked = state && state.progress > 0n ? Number(state.progress / FIXED) : 0;
    return { days: null, left: null, fraction: null, banked: banked > 0 ? `${focusDays(banked)} of progress saved for the next focus` : null };
  }
  const completion = state && baseDurationDays != null ? focusCompletion(state, baseDurationDays) : null;
  if (!completion) return { days: null, left: null, fraction: null, banked: null };
  return {
    days: `· Day ${Math.min(completion.elapsedDays + 1, Math.ceil(completion.totalDays))} of ${enTwoDecimals.format(completion.totalDays)}`,
    left: completion.daysLeft == null ? { value: `${Math.round(completion.fraction * 100)}%`, unit: 'complete', estimated: false }
      : { value: String(completion.daysLeft), unit: completion.daysLeft === 1 ? 'day left' : 'days left', estimated: true },
    fraction: completion.fraction,
    banked: null,
  };
}

export function activeFocus(order: { current: string | null; continuous: string | null } | null | undefined): { database: FocusDatabase; token: string } | null {
  if (order?.current != null) return { database: 'national', token: order.current };
  if (order?.continuous != null) return { database: 'continuous', token: order.continuous };
  return null;
}

export interface FocusProgressState {
  progress: bigint;
  dailyGain: bigint | null;
}

export function focusProgressAt(history: FocusHistory, hour: number): FocusProgressState | null {
  const index = spanAt(history.progress, hour);
  if (index < 0) return null;
  const run = history.progress[index];
  const progress = BigInt(run.progressRaw);
  const previous: FocusProgress | undefined = history.progress[index - 1];
  const current = focusAt(history, hour)?.current ?? null;
  const contiguous = previous && previous.lastHour + 1 === run.firstHour && current !== null && focusAt(history, previous.lastHour)?.current === current;
  const gain = contiguous ? progress - BigInt(previous.progressRaw) : null;
  return { progress, dailyGain: gain !== null && gain > 0n ? gain : null };
}

export interface FocusCompletion {
  fraction: number;
  elapsedDays: number;
  totalDays: number;
  daysLeft: number | null;
}

export function focusCompletion(state: FocusProgressState, baseDurationDays: number): FocusCompletion | null {
  if (!Number.isFinite(baseDurationDays) || baseDurationDays <= 0) return null;
  const total = BigInt(Math.round(baseDurationDays * 100_000));
  const done = state.progress < 0n ? 0n : state.progress > total ? total : state.progress;
  const remaining = total - done;
  return {
    fraction: Number((done * 10_000n) / total) / 10_000,
    elapsedDays: Number(done / FIXED),
    totalDays: baseDurationDays,
    daysLeft: state.dailyGain ? Number((remaining + state.dailyGain - 1n) / state.dailyGain) : null,
  };
}

export function validateFocusReply(input: unknown): FocusReply {
  const reply = input as FocusReply;
  let last = -Infinity;
  let lastProgress = -Infinity;
  let lastMissing = -Infinity;
  const valid = !!reply && typeof reply === 'object' && Array.isArray(reply.history?.segments) && Array.isArray(reply.history?.progress)
    && Array.isArray(reply.artwork) && Array.isArray(reply.warnings) && Array.isArray(reply.missing)
    && reply.missing.every(span => {
      const ok = Number.isInteger(span.firstHour) && Number.isInteger(span.lastHour) && span.firstHour > lastMissing && span.lastHour >= span.firstHour && typeof span.reason === 'string';
      lastMissing = span.lastHour;
      return ok;
    })
    && reply.history.segments.length <= 100000 && reply.history.segments.every(segment => {
      const ok = Number.isInteger(segment.firstHour) && Number.isInteger(segment.lastHour) && segment.firstHour > last && segment.lastHour >= segment.firstHour
        && Array.isArray(segment.order.completed) && segment.order.completed.every((entry, index) => entry.ordinal === index
          && (entry.completedByHour === null || (Number.isInteger(entry.completedByHour) && entry.completedByHour <= segment.firstHour)));
      last = segment.lastHour;
      return ok;
    })
    && reply.history.progress.length <= 1_000_000 && reply.history.progress.every(run => {
      const ok = Number.isInteger(run.firstHour) && Number.isInteger(run.lastHour) && run.firstHour > lastProgress && run.lastHour >= run.firstHour
        && typeof run.progressRaw === 'string' && /^-?\d{1,19}$/.test(run.progressRaw);
      lastProgress = run.lastHour;
      return ok;
    })
    && reply.artwork.every(artworkEntry)
    && (reply.staticText === undefined || Array.isArray(reply.staticText) && reply.staticText.every(item => artworkEntry(item) && item.icon === null))
    && Array.isArray(reply.continuousStaticText) && reply.continuousStaticText.every(item => artworkEntry(item) && item.icon === null);
  if (!valid) throw new Error('Focus history does not match this country and recording.');
  keepDrawableIcons(reply.artwork);
  return reply;
}

export function validateFocusArtworkReply(value: unknown, params: FocusArtworkParams): FocusArtworkReply {
  const reply = value as FocusArtworkReply;
  if (!reply || reply.branchId !== params.branchId || reply.hour !== params.hour || typeof reply.tag !== 'string' || (typeof params.country === 'string' && reply.tag !== params.country) || reply.database !== params.database
    || !Array.isArray(reply.items) || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('Focus artwork does not match this country and hour.');
  }
  const wanted = new Set(params.tokens), found = new Set<string>();
  for (const item of reply.items) {
    if (!item || !artworkEntry(item.artwork) || (item.reason !== null && typeof item.reason !== 'string')
      || !wanted.has(item.artwork.token) || found.has(item.artwork.token)) throw new Error('Invalid dated focus artwork.');
    found.add(item.artwork.token);
    const icon = drawableIcon(item.artwork.icon);
    if (item.artwork.icon !== null && icon === null) item.reason ??= 'The returned icon could not be displayed.';
    item.artwork.icon = icon;
  }
  if (found.size !== wanted.size) throw new Error('Dated focus artwork omitted requested tokens.');
  return reply;
}
