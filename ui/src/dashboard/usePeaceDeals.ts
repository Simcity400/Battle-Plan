import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useProduct } from './products.ts';
import { validatePeaceDealsReply } from './peaceDealsData.ts';
import type { PeaceDealsReply } from '../generated/dashboard';
import { productRequest } from './request.ts';

export function usePeaceDeals(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'peace-deals', params: { branchId: track.branch.id } }) : null;
  const reply = useProduct<PeaceDealsReply>(runtime.client, request, active, validatePeaceDealsReply, track?.source);
  return { track, data: reply.data, error: reply.error, retry: reply.retry };
}
