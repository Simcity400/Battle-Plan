import { asFailure, type Failure } from '../dashboard/queryError.ts';
import { HOUR_PACE_MS } from '../dashboard/playback.ts';
import type { PlaybackSnapshot } from '../dashboard/playback';

/** One reply: what shows from `hour` through `through` (a reply that says it holds). A `partial` one
 * was answered while its preparation was unfinished: it shows, and is read again until it is whole. */
export interface Answer<T> { hour: number; through: number; value: T; partial?: boolean; landed?: number }

export interface AheadOptions<T> {
  /** Reads the answers at `hours` (ascending); a product that reads one hour per request gets one. */
  /** `restart`: asked by an explicit retry, which may restart stopped preparation. */
  read: (hours: readonly number[], signal: AbortSignal, restart: boolean) => Promise<readonly Answer<T>[]>;
  /** Hours per request. */
  batch: number;
  /** Requests at once. */
  inFlight: number;
  /** An answer or a failure landed. */
  told: () => void;
}

/** How far ahead of the playhead answers are read, in wall time. */
export const AHEAD_MS = 1500;
const RETRY_MS = 1000;

/** Answers read ahead of the playhead, so the hour a frame shows is already here: while
 * playing, the hours every tick will land on (a lattice from where playing began, so a moving
 * playhead asks for each hour once) through `AHEAD_MS` ahead; paused, the hour shown. A
 * frame shows the newest answer at or before its hour (`at`), which says the hour it answers. */
export class AheadReads<T> {
  private answers: Answer<T>[] = [];
  private asking = new Set<number>();
  private failed = new Map<number, { failure: Failure; at: number }>();
  private readonly controllers = new Set<AbortController>();
  private requests = 0;
  private anchor: string | null = null;
  private lattice = { from: 0, step: 1 };
  private readonly options: AheadOptions<T>;
  private bounds: { first: number; last: number } | null = null;
  // A wake due for an hour shown that is partial or failed: paused, nothing else asks again.
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  // Set by `retry` for the reads it causes; automatic retries never restart preparation.
  private restartNext = false;

  constructor(options: AheadOptions<T>) { this.options = options; }

  /** Drops everything: another selection or source. */
  reset(): void {
    if (this.retryTimer !== null) clearTimeout(this.retryTimer);
    this.retryTimer = null;
    for (const controller of this.controllers) controller.abort();
    this.controllers.clear();
    this.answers = []; this.asking.clear(); this.failed.clear(); this.requests = 0; this.anchor = null;
  }

  /** Asks again at once for every hour that failed. */
  retry(): void { this.failed.clear(); this.restartNext = true; this.options.told(); }

  /** Reads what the next ticks will show, `hour` being the clock's hour as the reader counts it.
   * Cheap when nothing new is due: called every frame. */
  plan(clock: PlaybackSnapshot, shown: number, first: number, last: number): void {
    this.bounds = { first, last };
    const anchor = `${clock.seekSequence}:${clock.playing}:${clock.hoursPerSecond}`;
    if (anchor !== this.anchor) {
      this.anchor = anchor;
      const step = clock.playing ? Math.max(1, Math.round(clock.hoursPerSecond * HOUR_PACE_MS / 1000)) : 1;
      this.lattice = { from: shown, step };
    }
    const { from, step } = this.lattice;
    const hour = Math.max(first, Math.min(last, shown));
    // The newest answer before the playhead stays: it is what shows until the next lands.
    const keep = this.latestIndex(hour);
    if (keep > 0) this.answers.splice(0, keep);
    const wanted: number[] = [];
    const count = clock.playing ? Math.ceil(AHEAD_MS / HOUR_PACE_MS) + 1 : 1;
    const base = clock.playing ? from + Math.floor((hour - from) / step) * step : hour;
    const now = performance.now();
    for (let k = 0; k < count; k++) {
      const at = Math.min(last, Math.max(first, base + k * step));
      if (wanted.includes(at) || this.asking.has(at) || this.covers(at, now)) continue;
      const failure = this.failed.get(at);
      if (failure && now - failure.at < RETRY_MS) { this.wakeIn(failure.at + RETRY_MS - now); continue; }
      wanted.push(at);
    }
    // A retry's restart waits for a free slot: it is spent only by reads that start.
    while (wanted.length && this.requests < this.options.inFlight) {
      const restart = this.restartNext;
      this.restartNext = false;
      this.ask(wanted.splice(0, this.options.batch), restart);
    }
  }

  /** The newest answer at or before `hour`, and whether it answers that hour itself. */
  at(hour: number): { answer: Answer<T> | undefined; exact: boolean; failure?: Failure } {
    const index = this.latestIndex(hour);
    const answer = index >= 0 ? this.answers[index] : undefined;
    const exact = !!answer && hour <= answer.through;
    let failure: Failure | undefined;
    if (!exact) for (const [at, failed] of this.failed) if (at <= hour && (!answer || at > answer.hour)) { failure = failed.failure; break; }
    return { answer, exact, failure };
  }

  private latestIndex(hour: number): number {
    let low = 0, high = this.answers.length - 1, found = -1;
    while (low <= high) {
      const middle = (low + high) >> 1;
      if (this.answers[middle].hour <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
    }
    return found;
  }

  /** Whether an answer holds `hour`; a partial one only until it is due to be read again. */
  private covers(hour: number, now: number): boolean {
    const index = this.latestIndex(hour);
    const answer = index >= 0 ? this.answers[index] : undefined;
    if (!answer || hour > answer.through) return false;
    if (!answer.partial) return true;
    const due = (answer.landed ?? 0) + RETRY_MS;
    if (now < due) { this.wakeIn(due - now); return true; }
    return false;
  }

  private wakeIn(ms: number): void {
    if (this.retryTimer !== null) return;
    this.retryTimer = setTimeout(() => { this.retryTimer = null; this.options.told(); }, Math.max(0, ms) + 16);
  }

  private ask(hours: readonly number[], restart: boolean): void {
    const controller = new AbortController();
    this.controllers.add(controller);
    this.requests++;
    for (const hour of hours) { this.asking.add(hour); this.failed.delete(hour); }
    const done = () => { this.requests--; this.controllers.delete(controller); for (const hour of hours) this.asking.delete(hour); };
    this.options.read(hours, controller.signal, restart).then(answers => {
      if (controller.signal.aborted) return;
      done();
      const landed = performance.now();
      for (const answer of answers) this.insert(answer.partial ? { ...answer, landed } : answer);
      this.options.told();
    }, error => {
      if (controller.signal.aborted) return;
      done();
      const failure = { failure: asFailure(error), at: performance.now() };
      for (const hour of hours) this.failed.set(hour, failure);
      this.options.told();
    });
  }

  private insert(answer: Answer<T>): void {
    if (this.bounds && answer.through < this.bounds.first) return;
    const index = this.latestIndex(answer.hour);
    if (index >= 0 && this.answers[index].hour === answer.hour) this.answers[index] = answer;
    else this.answers.splice(index + 1, 0, answer);
  }
}
