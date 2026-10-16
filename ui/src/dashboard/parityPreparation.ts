import { useCallback, useState } from 'react';

/** Every real recording window intersecting the selected history. */
export function parityWindowIndices(windows: readonly { firstHour: number; lastHour: number }[] | undefined, first: number, last: number) {
  return (windows ?? []).flatMap((window, index) => window.firstHour <= last && window.lastHour >= first ? [index] : []);
}

export function parityWindowCoverage<T>(wanted: readonly number[], data: ReadonlyMap<number, T>, errors: ReadonlyMap<number, string>) {
  const outcomes = wanted.map(index => ({ index, status: errors.has(index) ? 'failed' : data.has(index) ? 'complete' : 'pending',
    ...(errors.has(index) ? { error: errors.get(index) } : {}) }));
  return { complete: outcomes.every(row => row.status === 'complete'),
    error: outcomes.find(row => row.status === 'failed')?.error ?? null, outcomes };
}

/** Used only under the build-mode guard, after the normal visual capture. */
export function useParityPreparation() {
  const [requested, setRequested] = useState(false);
  const prepare = useCallback(() => setRequested(true), []);
  return { requested, prepare };
}
