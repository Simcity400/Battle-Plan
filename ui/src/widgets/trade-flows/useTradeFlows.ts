import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import { contributionExtent } from './tradeDrill';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import { resolveCountries } from '../../dashboard/groups';
import type { ResolvedCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { ProductRequest } from '../../dashboard/client';
import { useProductWindows } from '../../dashboard/productWindows';
import { WINDOW_HOURS, windowsOf } from '../../dashboard/heldWindows';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { decodeTrade, mergeTrade, resourceName, tradeScope, validateTradeFlowsReply } from './tradeData';
import type { TradeFlowsReply } from '../../generated/dashboard';

export function tradeFlowsRequest(track: Pick<BoundTrack, 'source' | 'branch'>, tags: readonly string[] | null, from: number, to: number, window: number): ProductRequest {
  const first = window * WINDOW_HOURS;
  return { sourceId: track.source.id, revision: track.source.revision, product: 'trade-flows',
    params: { branchId: track.branch.id, countries: tags?.map(countryFromKey) ?? null, window, from: Math.max(from, first), to: Math.min(to, first + WINDOW_HOURS - 1) } };
}

export function useTradeWindows(client: DashboardClient, track: BoundTrack | undefined, tags: readonly string[] | null, from: number, to: number,
  enabled: boolean, active: boolean, source?: DashboardSource) {
  const wanted = useMemo(() => enabled ? windowsOf(from, to) : [], [enabled, from, to]);
  const identity = track && enabled ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tags, from, to]) : null;
  const held = useProductWindows<TradeFlowsReply>(client, identity, window => tradeFlowsRequest(track!, tags, from, to, window), wanted, active, validateTradeFlowsReply, source);
  const complete = wanted.length > 0 && wanted.every(window => held.data.has(window));
  const data = useMemo(() => complete ? mergeTrade(wanted.map(window => held.data.get(window)!)) ?? undefined : undefined, [complete, wanted, held.data]);
  const error = wanted.map(window => held.errors.get(window)).find(failure => failure !== undefined);
  return { data, error, retry: held.retry };
}

export function tradeExtent(widget: WidgetSpec, track: BoundTrack | undefined, tracks: readonly BoundTrack[]): [number, number] {
  if (!track) return [0, 1];
  const saved = contributionExtent(widget.options?.tradeExtent, track.branch);
  if (saved) return saved;
  const window = widget.time.mode === 'range' ? resolveWindow(widget, track, [...tracks], track.branch.lastHour) : null;
  return window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour];
}

export function useTradeFlows(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const marketView = widget.options?.view === 'market';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const grouped = widget.countries.kind === 'group';
  const all = widget.countries.kind === 'all';
  const groups = useCountryGroups(runtime, track, active && (grouped || marketView && !all));
  const fallback = defaultCountryKey(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const resolved = useMemo<ResolvedCountries | null>(() => all ? null
    : resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER), [all, widget.countries, fallback, groups.data, lastHour]);
  const tags = useMemo(() => resolved ? [...resolved.tags].sort() : null, [resolved]);
  const [from, to] = tradeExtent(widget, track, runtime.tracks);
  const ready = track && !marketView && (all || (tags && tags.length > 0));
  const product = useTradeWindows(runtime.client, track, tags, from, to, !!ready, active, track?.source);
  const data = marketView ? undefined : product.data;
  const scope = useMemo(() => data ? tradeScope(decodeTrade(data), widget.countries, resolved, groups.data, lastHour,
    widget.options?.between === 'between') : null, [data, widget.countries, widget.options?.between, resolved, groups.data, lastHour]);
  return { track, all, resolved, groups, scope, data, error: product.error, retry: product.retry, extent: [from, to] as [number, number] };
}

export function useTradeResourceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useTradeFlows(widget, runtime, true);
  return { resources: (data?.resources ?? []).filter(item => item.ordinal === 1).map(item => ({ value: item.token, label: resourceName(item), icon: item.icon })) };
}
