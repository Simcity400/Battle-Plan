import type { CountryParams, FocusDatabase, FocusHistory, FocusOrder, FocusSegment, FocusProgress, FocusReply, FocusArtworkParams, FocusArtworkReply } from '../../generated/dashboard';
import type { ProductRequest } from '../../dashboard/client';
import { artworkEntry, drawableIcon, keepDrawableIcons } from '../../dashboard/artwork.ts';

export const PRODUCT = 'national-focus';
const FIXED = 100_000n;

export function focusRequest(sourceId: string, revision: string, params: CountryParams): ProductRequest<CountryParams> {
  return { sourceId, revision, product: PRODUCT, params };
}

/** Binary search over disjoint, ordered spans. */
function spanAt<T extends { firstHour: number; lastHour: number }>(spans: readonly T[], hour: number): number {
  let low = 0, high = spans.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (spans[middle].firstHour <= hour) low = middle + 1;
    else high = middle;
  }
  return spans[low - 1] && hour <= spans[low - 1].lastHour ? low - 1 : -1;
}

/** Exact lookup over disjoint recorded segments; never carry a value over a gap. */
export function focusSegmentAt(history: FocusHistory, hour: number): FocusSegment | null {
  const index = spanAt(history.segments, hour);
  return index < 0 ? null : history.segments[index];
}

export function focusAt(history: FocusHistory, hour: number): FocusOrder | null {
  return focusSegmentAt(history, hour)?.order ?? null;
}

/** The focus a country is working on: its current national focus, else its continuous focus,
 * with the database its id is looked up in. One id can name a different focus in each. */
export function activeFocus(order: { current: string | null; continuous: string | null } | null | undefined): { database: FocusDatabase; token: string } | null {
  if (order?.current != null) return { database: 'national', token: order.current };
  if (order?.continuous != null) return { database: 'continuous', token: order.continuous };
  return null;
}

export interface FocusProgressState {
  /** Fixed5 days of recorded work toward the current focus. */
  progress: bigint;
  /** Fixed5 days gained at the last daily update, when the hour before it was recorded for the same focus. */
  dailyGain: bigint | null;
}

/** The recorded progress at `hour`, and the last day's gain when it is known without bridging a gap. */
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
  /** 0–1 of the focus's base duration, for the progress line. */
  fraction: number;
  /** Whole days of work recorded. */
  elapsedDays: number;
  totalDays: number;
  /** Days until completion at the last recorded daily rate; null when that rate is unknown. */
  daysLeft: number | null;
}

/** Progress toward a focus of `baseDurationDays`, from exact fixed5 operands. */
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

/** Shape checks on the product payload; identity is checked by the transport. Any
 * number of completed focuses is valid: a modded tree may have more than any base one. */
export function validateFocusReply(input: unknown): FocusReply {
  const reply = input as FocusReply;
  let last = -Infinity;
  let lastProgress = -Infinity;
  const valid = !!reply && typeof reply === 'object' && Array.isArray(reply.history?.segments) && Array.isArray(reply.history?.progress)
    && Array.isArray(reply.artwork) && Array.isArray(reply.warnings)
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
    // Authored duration metadata has no token count cap.
    && reply.artwork.every(artworkEntry)
    && (reply.staticText === undefined || Array.isArray(reply.staticText) && reply.staticText.every(item => artworkEntry(item) && item.icon === null))
    && Array.isArray(reply.continuousStaticText) && reply.continuousStaticText.every(item => artworkEntry(item) && item.icon === null);
  if (!valid) throw new Error('Focus history does not match this country and recording.');
  keepDrawableIcons(reply.artwork);
  return reply;
}

/** Authenticate the supplying read and preserve native per-item refusals. */
export function validateFocusArtworkReply(value: unknown, params: FocusArtworkParams): FocusArtworkReply {
  const reply = value as FocusArtworkReply;
  if (!reply || reply.branchId !== params.branchId || reply.hour !== params.hour || reply.tag !== params.tag || reply.database !== params.database
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
