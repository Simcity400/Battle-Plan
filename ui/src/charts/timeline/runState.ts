/** What a lane's recorded runs say at an hour, honestly about the hours the
 * recording lacks: held since when (or held before a day, when nothing was
 * recorded the hour before it started), ended when (or ended by a day, when the
 * end fell in an unrecorded stretch), or unknown (last recorded when, with the
 * hours after it not recorded yet). Shared by every lane timeline that names a
 * holding's start and end: national spirits, faction memberships and subjects. */

import { inSpans } from './layout.ts';
import type { BandStep } from './layout';

/** Hours, both included. */
export interface HourRange { firstHour: number; lastHour: number }

/** How many of `runs` cover each hour, as band steps (`TimelineSpan.band`) where at
 * least one does: a step per stretch of one count, in hour order. Runs may overlap
 * and come in any order. O(n log n) in the runs. */
export function coverCounts(runs: readonly HourRange[]): BandStep[] {
  const changes = new Map<number, number>();
  for (const run of runs) {
    if (run.lastHour < run.firstHour) continue;
    changes.set(run.firstHour, (changes.get(run.firstHour) ?? 0) + 1);
    changes.set(run.lastHour + 1, (changes.get(run.lastHour + 1) ?? 0) - 1);
  }
  const hours = [...changes.keys()].sort((a, b) => a - b);
  const steps: BandStep[] = [];
  let count = 0;
  hours.forEach((hour, index) => {
    count += changes.get(hour)!;
    const next = hours[index + 1];
    if (count > 0 && next != null) steps.push({ firstHour: hour, lastHour: next - 1, value: count });
  });
  return steps;
}

/** The value every band is drawn full thickness at (`TimelineProps.bandMax`): the
 * largest any step reaches, at least one. A loop, not a spread, so any number of steps
 * fits (a spread into `Math.max` overflows the stack past about 100,000 arguments). */
export function bandMaxOf(bands: Iterable<readonly BandStep[]>): number {
  let largest = 1;
  for (const steps of bands) for (const step of steps) if (step.value > largest) largest = step.value;
  return largest;
}

/** `steps` without the hours `spans` cover (such as hours a roster was not known),
 * each step cut around them. Both in hour order; linear in the two. */
export function withoutHours<S extends HourRange>(steps: readonly S[], spans: readonly HourRange[]): S[] {
  const result: S[] = [];
  let index = 0;
  for (const step of steps) {
    let from = step.firstHour;
    while (index < spans.length && spans[index].lastHour < from) index++;
    for (let at = index; at < spans.length && spans[at].firstHour <= step.lastHour && from <= step.lastHour; at++) {
      const span = spans[at];
      if (span.firstHour > from) result.push({ ...step, firstHour: from, lastHour: span.firstHour - 1 });
      from = Math.max(from, span.lastHour + 1);
    }
    if (from <= step.lastHour) result.push({ ...step, firstHour: from, lastHour: step.lastHour });
  }
  return result;
}

/** Whether every hour from `from` to `to` (both included) falls in `spans` (sorted,
 * merged or not). An empty range is covered. */
export function coveredBy(from: number, to: number, spans: readonly HourRange[]): boolean {
  return spanCoverage(spans)(from, to);
}

/** Coverage queries in hour order. Keep both the cursor and the end already
 * covered: one long span may cover several gaps, and touching spans may join. */
function spanCoverage(spans: readonly HourRange[]): (from: number, to: number) => boolean {
  let index = 0;
  let through = Number.NEGATIVE_INFINITY;
  return (from, to) => {
    if (from > to) return true;
    let next = Math.max(from, through + 1);
    while (next <= to && index < spans.length) {
      const span = spans[index];
      if (span.firstHour > next) break;
      through = Math.max(through, span.lastHour);
      index++;
      next = Math.max(next, through + 1);
    }
    return next > to;
  };
}

/** One holding's runs (in hour order) joined into stretches where nothing recorded
 * says it stopped: the next run starts the hour after, or every hour between them was
 * not recorded, and `joinable` agrees (such as the same relation). A stretch is what
 * its state is said of ("Ended by", "Still a member"), so a holding continued across
 * an unrecorded stretch is never called ended; its bars still show the gap. */
export function bridgeGaps<R extends HourRange>(runs: readonly R[], unobserved: readonly HourRange[] = [],
  joinable: (earlier: R, later: R) => boolean = () => true): { firstHour: number; lastHour: number; runs: R[] }[] {
  // Native histories already come sorted; keep their pass linear. Other callers
  // may still give spans out of order, as before.
  const sorted = unobserved.every((span, index) => !index || unobserved[index - 1].firstHour <= span.firstHour);
  const spans = sorted ? unobserved : [...unobserved].sort((a, b) => a.firstHour - b.firstHour);
  const covered = spanCoverage(spans);
  const stretches: { firstHour: number; lastHour: number; runs: R[] }[] = [];
  for (const run of runs) {
    const last = stretches.at(-1);
    const previous = last?.runs.at(-1);
    if (last && previous && joinable(previous, run) && covered(last.lastHour + 1, run.firstHour - 1)) {
      last.lastHour = Math.max(last.lastHour, run.lastHour);
      last.runs.push(run);
    } else stretches.push({ firstHour: run.firstHour, lastHour: run.lastHour, runs: [run] });
  }
  return stretches;
}

/** The stretch holding an hour, else the last before it, else the first. */
export function stretchAt<S extends HourRange>(stretches: readonly S[], hour: number): S | undefined {
  let found: S | undefined;
  for (const stretch of stretches) if (stretch.firstHour <= hour) found = stretch;
  return found ?? stretches[0];
}

/** The stretches of consecutive hours `steps` cover, each with its steps. */
export function stretchesOf<S extends HourRange>(steps: readonly S[]): { firstHour: number; lastHour: number; steps: S[] }[] {
  const stretches: { firstHour: number; lastHour: number; steps: S[] }[] = [];
  for (const step of steps) {
    const last = stretches.at(-1);
    if (last && last.lastHour + 1 === step.firstHour) { last.lastHour = step.lastHour; last.steps.push(step); }
    else stretches.push({ firstHour: step.firstHour, lastHour: step.lastHour, steps: [step] });
  }
  return stretches;
}

/** A holding's state at an hour: held (since when), ended (when; with `lastSeen`
 * when the end fell in an unrecorded stretch, so only "by" that hour is known),
 * unknown (the hours after it was last seen are not recorded yet: last seen when),
 * or not yet held. */
export type RunState = { kind: 'held'; since: number } | { kind: 'ended'; at: number; lastSeen?: number } | { kind: 'unknown'; lastSeen: number } | { kind: 'none' };

/** The state at `hour` of a holding recorded as `runs` (in hour order). A run
 * covers its last recorded hour up to the next one. */
export function runState(runs: readonly HourRange[], hour: number, unobserved: readonly HourRange[] = []): RunState {
  let last: HourRange | undefined;
  for (const run of runs) if (run.firstHour <= hour) last = run;
  if (!last) return { kind: 'none' };
  if (hour < last.lastHour + 1) return { kind: 'held', since: last.firstHour };
  // A holding whose next hours were not recorded may still be held: unknown, not an
  // end, until the first recorded hour after them shows it gone.
  const through = unrecordedThrough(unobserved, last.lastHour + 1);
  if (through != null) return hour <= through ? { kind: 'unknown', lastSeen: last.lastHour } : { kind: 'ended', at: through + 1, lastSeen: last.lastHour };
  return { kind: 'ended', at: last.lastHour + 1 };
}

/** The last hour of the unrecorded stretch `spans` (in any order) cover from `from` on,
 * touching and overlapping spans joined (a recorded gap, then hours still being
 * prepared); null when `from` is recorded. */
function unrecordedThrough(spans: readonly HourRange[], from: number): number | null {
  const sorted = spans.every((span, index) => !index || spans[index - 1].firstHour <= span.firstHour) ? spans : [...spans].sort((a, b) => a.firstHour - b.firstHour);
  let through = from - 1;
  for (const span of sorted) {
    if (span.firstHour > through + 1) break;
    through = Math.max(through, span.lastHour);
  }
  return through >= from ? through : null;
}

/** Whether nothing was recorded the hour before a run starts (the branch start or
 * an unrecorded stretch): the holding then began at an unknown time up to that
 * hour, not at it. */
export function startUnseen(firstHour: number, branchFirstHour: number, unobserved: readonly HourRange[] = []): boolean {
  return firstHour <= branchFirstHour || inSpans(unobserved, firstHour - 1);
}

/** The words a timeline uses for each state, each given the hour it names. */
export interface RunWords {
  /** Held, and seen starting. */
  since: (hour: number) => string;
  /** Held, but its start was not recorded: held by then. */
  before: (hour: number) => string;
  ended: (hour: number) => string;
  /** Ended in an unrecorded stretch: gone by then. */
  endedBy: (hour: number) => string;
  /** The hours after it are not recorded yet. */
  lastSeen: (hour: number) => string;
}

/** The runs' state at `hour` in words, or null before the first run. */
export function describeRun(runs: readonly HourRange[], hour: number, words: RunWords, unobserved: readonly HourRange[] = [],
  branchFirstHour = Number.NEGATIVE_INFINITY): string | null {
  const state = runState(runs, hour, unobserved);
  switch (state.kind) {
    case 'none': return null;
    case 'ended': return state.lastSeen == null ? words.ended(state.at) : words.endedBy(state.at);
    case 'unknown': return words.lastSeen(state.lastSeen);
    case 'held': return startUnseen(state.since, branchFirstHour, unobserved) ? words.before(state.since) : words.since(state.since);
  }
}
