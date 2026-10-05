import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useProduct } from './products';
import { validateRaidsReply } from './raids';
import type { RaidsParams } from '../generated/dashboard';

export function useRaids(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, params: Omit<RaidsParams, 'branchId'>) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'raids', params: { ...params, branchId: track.branch.id } } : null;
  return { track, ...useProduct(runtime.client, request, active, validateRaidsReply) };
}
