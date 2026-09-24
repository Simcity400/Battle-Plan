import type { CountryParams, FocusHistory, FocusOrder, FocusProgress, FocusReply } from '../../generated/dashboard';
import type { ProductRequest } from '../../dashboard/client';

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
export function focusAt(history: FocusHistory, hour: number): FocusOrder | null {
  const index = spanAt(history.segments, hour);
  return index < 0 ? null : history.segments[index].order;
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

/** Shape checks on the product payload; identity is checked by the transport. */
export function validateFocusReply(input: unknown): FocusReply {
  const reply = input as FocusReply;
  let last = -Infinity;
  let lastProgress = -Infinity;
  const valid = !!reply && typeof reply === 'object' && Array.isArray(reply.history?.segments) && Array.isArray(reply.history?.progress)
    && Array.isArray(reply.artwork) && Array.isArray(reply.warnings)
    && reply.history.segments.length <= 100000 && reply.history.segments.every(segment => {
      const ok = Number.isInteger(segment.firstHour) && Number.isInteger(segment.lastHour) && segment.firstHour > last && segment.lastHour >= segment.firstHour
        && segment.order.completed.length <= 512 && segment.order.completed.every((entry, index) => entry.ordinal === index
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
    && reply.artwork.length <= 2048 && reply.artwork.every(({ icon, baseDurationDays }) => (baseDurationDays === null || (Number.isFinite(baseDurationDays) && baseDurationDays >= 0))
      && (icon === null || (icon.length <= 128 * 1024 && /^data:image\/png;base64,[A-Za-z0-9+/]+=*$/.test(icon))));
  if (!valid) throw new Error('Focus history does not match this country and recording.');
  return reply;
}
