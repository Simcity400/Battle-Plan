
import { inSpans } from './layout.ts';
import type { BandStep } from './layout';

export interface HourRange { firstHour: number; lastHour: number }

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

export function bridgeGaps<R extends HourRange>(runs: readonly R[], unobserved: readonly HourRange[] = [],
  joinable: (earlier: R, later: R) => boolean = () => true): { firstHour: number; lastHour: number; runs: R[] }[] {
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

export function stretchesOf<S extends HourRange>(steps: readonly S[]): { firstHour: number; lastHour: number; steps: S[] }[] {
  const stretches: { firstHour: number; lastHour: number; steps: S[] }[] = [];
  for (const step of steps) {
    const last = stretches.at(-1);
    if (last && last.lastHour + 1 === step.firstHour) { last.lastHour = step.lastHour; last.steps.push(step); }
    else stretches.push({ firstHour: step.firstHour, lastHour: step.lastHour, steps: [step] });
  }
  return stretches;
}

export type RunState = { kind: 'held'; since: number } | { kind: 'ended'; at: number; lastSeen?: number } | { kind: 'unknown'; lastSeen: number } | { kind: 'none' };

export function runState(runs: readonly HourRange[], hour: number, unobserved: readonly HourRange[] = []): RunState {
  let last: HourRange | undefined;
  for (const run of runs) if (run.firstHour <= hour) last = run;
  if (!last) return { kind: 'none' };
  if (hour < last.lastHour + 1) return { kind: 'held', since: last.firstHour };
  const through = unrecordedThrough(unobserved, last.lastHour + 1);
  if (through != null) return hour <= through ? { kind: 'unknown', lastSeen: last.lastHour } : { kind: 'ended', at: through + 1, lastSeen: last.lastHour };
  return { kind: 'ended', at: last.lastHour + 1 };
}

function unrecordedThrough(spans: readonly HourRange[], from: number): number | null {
  const sorted = spans.every((span, index) => !index || spans[index - 1].firstHour <= span.firstHour) ? spans : [...spans].sort((a, b) => a.firstHour - b.firstHour);
  let through = from - 1;
  for (const span of sorted) {
    if (span.firstHour > through + 1) break;
    through = Math.max(through, span.lastHour);
  }
  return through >= from ? through : null;
}

export function startUnseen(firstHour: number, branchFirstHour: number, unobserved: readonly HourRange[] = []): boolean {
  return firstHour <= branchFirstHour || inSpans(unobserved, firstHour - 1);
}

export interface RunWords {
  since: (hour: number) => string;
  before: (hour: number) => string;
  ended: (hour: number) => string;
  endedBy: (hour: number) => string;
  lastSeen: (hour: number) => string;
}

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
