import type { FocusReply, FocusRequest, CountryFlagReply, CountryFlagRequest, DashboardClient, LandOwnersReply } from './client';
import { focusReplyBytes } from './focusData.ts';
import { ResourceCache } from './resourceCache.ts';

const caches = new WeakMap<DashboardClient, ReturnType<typeof createCache>>();
function createCache(client: DashboardClient) {
  const focuses = new ResourceCache<FocusReply>(64, 32 * 1024 * 1024, focusReplyBytes);
  const owners = new ResourceCache<LandOwnersReply>(16, 4 * 1024 * 1024, value => JSON.stringify(value).length * 2);
  const flags = new ResourceCache<CountryFlagReply>(64, 8 * 1024 * 1024, value => value.flags.reduce((bytes, flag) => bytes + flag.pixels.length, 0) + 2048);
  return {
    focus: (query: FocusRequest, signal: AbortSignal) => focuses.get(JSON.stringify([query.sourceId, query.revision, query.branchId, query.tag]), shared => client.focusHistory!(query, shared), signal),
    retryFocus: (sourceId: string) => focuses.deleteSource(sourceId),
    owners: (sourceId: string, revision: string, signal: AbortSignal) => owners.get(JSON.stringify([sourceId, revision]),
      shared => client.landOwners!(sourceId, revision, shared), signal),
    flags: (query: CountryFlagRequest, signal: AbortSignal) => flags.get(JSON.stringify([query.sourceId, query.revision, query.branchId, query.hour, query.size, [...query.tags].sort()]),
      shared => client.countryFlags!(query, shared), signal),
    retryFlags: (sourceId: string) => flags.deleteSource(sourceId),
    release: (sourceId: string) => { owners.deleteSource(sourceId); flags.deleteSource(sourceId); focuses.deleteSource(sourceId); },
  };
}
export function countryData(client: DashboardClient) {
  let cache = caches.get(client);
  if (!cache) { cache = createCache(client); caches.set(client, cache); }
  return cache;
}
