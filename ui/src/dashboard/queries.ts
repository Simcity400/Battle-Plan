import { useCallback, useMemo, useSyncExternalStore } from 'react';
import type { DashboardQuery } from './client';
import { QueryStore, queryKey } from './queryStore';
export { QueryStore, queryKey } from './queryStore';

export function useQueries(store: QueryStore, requests: DashboardQuery[], prefetch: DashboardQuery[] = []) {
  const keys = requests.map(queryKey).join('\n');
  const futureKeys = prefetch.map(queryKey).join('\n');
  const entries = useMemo(() => [...new Map(requests.map((request) => [queryKey(request), request])).values()].map((request) => store.entry(request)), [store, keys]);
  const future = useMemo(() => [...new Map(prefetch.map((request) => [queryKey(request), request])).values()].filter((request) => !requests.some((current) => queryKey(current) === queryKey(request)))
    .map((request) => store.entry(request)), [store, keys, futureKeys]);
  const subscribe = useCallback((listener: () => void) => {
    const release = [...entries.map((entry) => store.retain(entry, listener)), ...future.map((entry) => store.retain(entry, null))];
    return () => release.forEach((stop) => stop());
  }, [store, entries, future]);
  const snapshot = useCallback(() => entries.map((entry) => entry.version).join(':'), [entries]);
  useSyncExternalStore(subscribe, snapshot, snapshot);
  return entries.map((entry) => ({ key: entry.key, request: entry.request, ...entry.snapshot, retry: () => store.retry(entry) }));
}
