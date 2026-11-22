import { useGate, type Gate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
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
import { productRequest } from '../../dashboard/request.ts';

export function tradeFlowsRequest(gate: Pick<Gate, 'span'>, track: Pick<BoundTrack, 'source' | 'branch'>, tags: readonly string[] | null, from: number, to: number,
  window: number): ProductRequest | null {
  const first = window * WINDOW_HOURS;
  const span = gate.span(Math.max(from, first), Math.min(to, first + WINDOW_HOURS - 1));
  return span ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'trade-flows',
    params: { branchId: track.branch.id, countries: tags?.map(countryFromKey) ?? null, window, from: span.firstHour, to: span.lastHour } }) : null;
}

export function useTradeWindows(client: DashboardClient, gate: Pick<Gate, 'span'>, track: BoundTrack | undefined, tags: readonly string[] | null, from: number, to: number,
  enabled: boolean, active: boolean, source?: DashboardSource) {
  const covered = gate.span(from, to);
  const wanted = useMemo(() => enabled && covered ? windowsOf(covered.firstHour, covered.lastHour) : [], [enabled, covered?.firstHour, covered?.lastHour]);
  const identity = track && enabled && covered ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tags, from, to]) : null;
  const held = useProductWindows<TradeFlowsReply>(client, identity, window => tradeFlowsRequest(gate, track!, tags, from, to, window)!, wanted, active, validateTradeFlowsReply, source);
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
  const gate = useGate(runtime, widget, SPEC, active);
  const product = useTradeWindows(runtime.client, gate, track, tags, from, to, !!ready, active, track?.source);
  const data = marketView ? undefined : product.data;
  const scope = useMemo(() => data ? tradeScope(decodeTrade(data), widget.countries, resolved, groups.data, lastHour,
    widget.options?.between === 'between') : null, [data, widget.countries, widget.options?.between, resolved, groups.data, lastHour]);
  return { track, all, resolved, groups, scope, data, error: product.error, retry: product.retry, extent: [from, to] as [number, number] };
}

export function useTradeResourceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useTradeFlows(widget, runtime, true);
  return { resources: (data?.resources ?? []).filter(item => item.ordinal === 1).map(item => ({ value: item.token, label: resourceName(item), icon: item.icon })) };
}
