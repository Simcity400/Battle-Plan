import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import type { PeaceDealsReply } from '../generated/dashboard';
import { useProduct } from './products';
import { validatePeaceDealsReply } from './peaceDealsData.ts';

export function usePeaceDeals(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'peace-deals', params: { branchId: track.branch.id } } : null;
  return { track, ...useProduct<PeaceDealsReply>(runtime.client, request, active, validatePeaceDealsReply) };
}
