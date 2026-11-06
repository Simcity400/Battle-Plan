import { failureText } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useState } from 'react';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import type { ProductionLineDetailReply, ProductionLineHourReply, ProductionLinesReply, ProductionLineBreakdownReply } from '../../generated/dashboard';
import { datedCountrySpans } from '../../dashboard/datedCountry';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { groupNumbers } from '../../dashboard/groupsReply';
import { NumericReply } from '../../dashboard/numericReply';
import type { CountryGroupReply } from '../../generated/dashboard';
import { joinHour, validateDetailReply, validateHourReply, validateProductionReply, validateBreakdownReply } from './productionData';

/** The country's published production history, refreshed as preparation advances. */
export function useProductionLines(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'production-lines',
    params: { branchId: track.branch.id, tag } } : null;
  const lines = useProduct<ProductionLinesReply>(runtime.client, request, active, validateProductionReply, track?.source);
  return { track, tag, ...lines };
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

/** The live table's reads at `hour` while playback runs: every line's at the first hour read,
 * then only the lines whose reads changed since the hour read before, joined to those held.
 * `hour` is the hour the reads answer, which trails the playhead by the read in flight. */
export function useLineHour(runtime: WidgetRuntime, track: BoundTrack | undefined, tag: string | undefined, hour: number | null, detailHours: number | undefined, active: boolean) {
  const lane = track && tag && detailHours ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag]) : null;
  const [held, setHeld] = useState<{ lane: string; reply: ProductionLineHourReply; detail: ProductionLineDetailReply } | null>(null);
  const holding = held && held.lane === lane ? held : null;
  // A later hour is read against the one held; the held hour asks again what it was asked.
  const since = !holding || hour === null || holding.reply.basis === null ? undefined : hour > holding.reply.hour ? holding.reply.hour
    : hour === holding.reply.hour ? holding.reply.since ?? undefined : undefined;
  const basis = since === undefined || !holding ? undefined : holding.reply.basis ?? undefined;
  const read = useFollowedProduct<ProductionLineHourReply>(runtime.client, track && tag && lane && hour !== null ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'production-line-detail', params: { branchId: track.branch.id, tag, hour, ...(since === undefined ? {} : { since }), ...(basis === undefined ? {} : { basis }) } } : null,
  ['hour', 'since', 'basis'], active, validateHourReply, track?.source);
  useEffect(() => {
    const reply = read.data;
    if (!reply || !lane || !detailHours) return;
    setHeld(previous => {
      const kept = previous?.lane === lane ? previous : null;
      if (kept?.reply === reply) return previous;
      // Lines read against another hour than the one held are not joined to it.
      if (reply.since !== null && (kept?.reply.hour !== reply.since || kept.reply.basis !== reply.basis)) return previous;
      return { lane, reply, detail: joinHour(kept?.detail ?? null, reply, detailHours) };
    });
  }, [read.data, lane, detailHours]);
  return { detail: holding?.detail, hour: holding?.reply.hour, error: read.error, retry: () => { setHeld(null); read.retry(); } };
}

export function useProductionCountry(selection: DatedCountrySelection | null, runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean) {
  const catalogue = useCountryGroups(runtime, track, active && !!selection);
  const retained = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const history = retained.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]), catalogue.data, groupNumbers);
  const spans = useMemo(() => selection && history && track && history.branchId === track.branch.id
    && selection.campaign === track.source.metadata.campaignKey && selection.branch === track.branch.id
    ? datedCountrySpans(history.countryIdentities, selection, track.branch.firstHour, track.branch.lastHour) : null,
  [selection, history, track?.source.metadata.campaignKey, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour]);
  return { spans, error: history ? undefined : catalogue.error, identityError: history ? catalogue.error : undefined, retry: catalogue.retry,
    reason: (!history && catalogue.error ? failureText(catalogue.error, 'Couldn’t load country.') : undefined) ?? (!history ? 'Loading country…'
    : !spans ? 'Saved country selection unavailable. Open the breakdown again.' : null) };
}
