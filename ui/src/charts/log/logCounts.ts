/** A filtered window's count is reusable after its supplying reads have all
 * happened. Partial counts apply only at the hour that counted them. */
export interface ObservedLogCount { count: number; atHour: number; completeBy: number }
export interface LogCountSample { key: string; count: number; completeBy: number }

export function logCountAt(observed: ObservedLogCount | undefined, hour: number | null, fallback: number): number {
  return observed && hour !== null && (hour === observed.atHour
    || (observed.atHour >= observed.completeBy && hour >= observed.completeBy)) ? observed.count : fallback;
}

/** Preserve the same map when nothing changed, so drawing a log cannot trigger
 * another render through its visible-row callback. */
export function rememberLogCounts(held: ReadonlyMap<string, ObservedLogCount>, samples: readonly LogCountSample[], hour: number): ReadonlyMap<string, ObservedLogCount> {
  let next: Map<string, ObservedLogCount> | undefined;
  for (const sample of samples) {
    const previous = held.get(sample.key);
    if (previous?.count === sample.count && previous.completeBy === sample.completeBy
      && (previous.atHour === hour || (previous.atHour >= previous.completeBy && hour >= previous.completeBy))) continue;
    next ??= new Map(held);
    next.set(sample.key, { count: sample.count, atHour: hour, completeBy: sample.completeBy });
  }
  return next ?? held;
}
