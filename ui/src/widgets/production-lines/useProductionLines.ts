import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import type { ProductionLineDetailReply, ProductionLinesReply } from '../../generated/dashboard';
import { validateDetailReply, validateProductionReply } from './productionData';

/** The widget's country and its production lines' history. */
export function useProductionLines(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'production-lines',
    params: { branchId: track.branch.id, tag } } : null;
  return { track, tag, ...useProduct<ProductionLinesReply>(runtime.client, request, active, validateProductionReply) };
}

/** The `wanted` windows of the country's line reads (`production-line-detail`), each
 * held while wanted. */
export function useLineDetail(runtime: WidgetRuntime, track: BoundTrack | undefined, tag: string | undefined, wanted: readonly number[], active: boolean) {
  // Structured, so no tag (a mod's tags may hold any character) can blur two requests.
  const identity = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag]) : null;
  return useProductWindows<ProductionLineDetailReply>(runtime.client, identity, window => ({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'production-line-detail', params: { branchId: track!.branch.id, tag: tag!, window } }), wanted, active, validateDetailReply);
}
