import { useMemo } from 'react';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import type { ProductionLineDetailReply, ProductionLinesReply, ProductionLineBreakdownReply } from '../../generated/dashboard';
import { datedCountrySpans } from '../../dashboard/datedCountry';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { groupNumbers } from '../../dashboard/groupsReply';
import { NumericReply } from '../../dashboard/numericReply';
import type { CountryGroupReply } from '../../generated/dashboard';
import { validateDetailReply, validateProductionReply, validateBreakdownReply } from './productionData';

/** The widget's country and its production lines' history. */
export function useProductionLines(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'production-lines',
    params: { branchId: track.branch.id, tag } } : null;
  return { track, tag, ...useProduct<ProductionLinesReply>(runtime.client, request, active, validateProductionReply) };
}

export function useLineBreakdown(runtime: WidgetRuntime, track: BoundTrack | undefined, tag: string | undefined, key: string | null, group: string, active: boolean, originHour?: number) {
  const request = track && tag && key ? { sourceId: track.source.id, revision: track.source.revision, product: 'production-line-breakdown',
    params: { branchId: track.branch.id, tag, key, group, ...(originHour === undefined ? {} : { originHour }) } } : null;
  return useProduct<ProductionLineBreakdownReply>(runtime.client, request, active, validateBreakdownReply);
}

/** The `wanted` windows of the country's line reads (`production-line-detail`), each
 * held while wanted. */
export function useLineDetail(runtime: WidgetRuntime, track: BoundTrack | undefined, tag: string | undefined, wanted: readonly number[], active: boolean) {
  // Structured, so no tag (a mod's tags may hold any character) can blur two requests.
  const identity = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag]) : null;
  return useProductWindows<ProductionLineDetailReply>(runtime.client, identity, window => ({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'production-line-detail', params: { branchId: track!.branch.id, tag: tag!, window } }), wanted, active, validateDetailReply);
}

export function useProductionCountry(selection: DatedCountrySelection | null, runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean) {
  const catalogue = useCountryGroups(runtime, track, active && !!selection);
  const retained = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const history = retained.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]), catalogue.data, groupNumbers);
  const spans = useMemo(() => selection && history && track && history.branchId === track.branch.id
    && selection.campaign === track.source.metadata.campaignKey && selection.branch === track.branch.id
    ? datedCountrySpans(history.countryIdentities, selection, track.branch.firstHour, track.branch.lastHour) : null,
  [selection, history, track?.source.metadata.campaignKey, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour]);
  return { spans, error: catalogue.error, retry: catalogue.retry, reason: catalogue.error ?? (!history ? 'Loading country…'
    : !spans ? 'Saved country selection unavailable. Open the breakdown again.' : null) };
}
