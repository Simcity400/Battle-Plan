import { useMemo } from 'react';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import { resolveCountries } from '../../dashboard/groups';
import type { ResolvedCountries } from '../../dashboard/groups';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { ProductRequest } from '../../dashboard/client';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { TradeFlowsReply } from '../../generated/dashboard';
import { decodeTrade, resourceName, tradeScope, validateTradeFlowsReply } from './tradeData';

/** The trade flows product for a branch: every country's trade (null), or the deals of
 * the holders that ever had one of `tags`. */
export function tradeFlowsRequest(track: Pick<BoundTrack, 'source' | 'branch'>, tags: readonly string[] | null): ProductRequest {
  return { sourceId: track.source.id, revision: track.source.revision, product: 'trade-flows', params: { branchId: track.branch.id, tags } };
}

/** The widget's countries (all countries, chosen ones or a group's members) and
 * the trade flows reply for them. Chosen countries narrow the product's scan to
 * their deals; a group counted at each hour is cut to its membership afterwards. */
export function useTradeFlows(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const grouped = widget.countries.kind === 'group';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryTag(track);
  const all = widget.countries.kind === 'all';
  const lastHour = track?.branch.lastHour ?? 0;
  const resolved = useMemo<ResolvedCountries | null>(() => all ? null
    : resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER), [all, widget.countries, fallback, groups.data, lastHour]);
  const tags = useMemo(() => resolved ? [...resolved.tags].sort() : null, [resolved]);
  const ready = track && (all || (tags && tags.length > 0));
  const request = ready ? tradeFlowsRequest(track, tags) : null;
  const product = useProduct<TradeFlowsReply>(runtime.client, request, active, validateTradeFlowsReply);
  const scope = useMemo(() => product.data ? tradeScope(decodeTrade(product.data), widget.countries, resolved, groups.data, lastHour,
    widget.options?.between === 'between') : null, [product.data, widget.countries, widget.options?.between, resolved, groups.data, lastHour]);
  return { track, all, resolved, groups, scope, ...product };
}

/** The Resources setting's choices: the campaign's own resources, named and pictured
 * as the game does, one per token (a repeated declaration is chosen with its token). */
export function useTradeResourceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useTradeFlows(widget, runtime, true);
  return { resources: (data?.resources ?? []).filter(item => item.ordinal === 1).map(item => ({ value: item.token, label: resourceName(item), icon: item.icon })) };
}
