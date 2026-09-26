import type { CountryGroupReply } from '../generated/dashboard';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { useProduct } from './products';

/** The branch's smart-group history, loaded once and shared through the product cache. */
export function useCountryGroups(runtime: Pick<WidgetRuntime, 'client'>, track: BoundTrack | undefined, active = true) {
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-groups', params: { branchId: track.branch.id } } : null;
  return useProduct<CountryGroupReply>(runtime.client, request, active);
}
