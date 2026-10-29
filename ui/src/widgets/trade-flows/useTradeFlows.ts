import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { TimeChartProps } from '../../charts/TimeChart';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { resolveWindow } from '../../dashboard/model';
import { contributionExtent } from './tradeDrill';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import { worldFilter, resolveCountries } from '../../dashboard/groups';
import type { ResolvedCountries } from '../../dashboard/groups';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { loadProduct, useFollowedProduct, useProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useViewRead } from '../../dashboard/viewRead';
import type { ProductRequest } from '../../dashboard/client';
import { NumericReply } from '../../dashboard/numericReply';
import type { MarketLendLeaseReply } from '../../generated/dashboard';
import { marketRequest, validateMarketReply } from './marketData';
import type { PreparedTradeReply } from './tradeData';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { mergePreparedTrade, decodeTrade, resourceName, tradeScope, validateTradeFlowsReply } from './tradeData';

/** The trade flows product for a branch: every country's trade (null), or the deals of
 * the holders that ever had one of `tags`. */
export function tradeFlowsRequest(track: Pick<BoundTrack, 'source' | 'branch'>, tags: readonly string[] | null, prepared?: Record<string, unknown>): ProductRequest {
  return { sourceId: track.source.id, revision: track.source.revision, product: 'trade-flows', params: { branchId: track.branch.id, tags, ...prepared } };
}

/** The widget's countries (all countries, chosen ones or a group's members) and
 * the trade flows reply for them. Chosen countries narrow the product's scan to
 * their deals; a group counted at each hour is cut to its membership afterwards. */
export function useTradeFlows(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, marketHour?: number) {
  const marketView = widget.options?.view === 'market';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const grouped = widget.countries.kind === 'group';
  const all = widget.countries.kind === 'all';
  const groups = useCountryGroups(runtime, track, active && (grouped || marketView && !all));
  const fallback = defaultCountryTag(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const selectionWindow = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const selectionHour = marketView ? marketHour ?? Math.floor(selectionWindow?.lastHour ?? lastHour) : lastHour;
  const resolved = useMemo<ResolvedCountries | null>(() => all ? null
    : resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER), [all, widget.countries, fallback, groups.data, lastHour]);
  const tags = useMemo(() => resolved ? [...resolved.tags].sort() : null, [resolved]);
  const ready = track && !marketView && (all || (tags && tags.length > 0));
  const key = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tags, widget.countries, widget.options?.between, widget.options?.measure]);
  const [viewport, setViewport] = useState<{ key: string; from: number; to: number; width: number }>();
  const [point, setPoint] = useState<{ key: string; hour: number | null }>();
  const [detail, setDetail] = useState<{ key: string; hour: number | null }>();
  const [keyboard, setKeyboard] = useState<{ key: string; hour: number | null; revision: number }>();
  const [navigationError, setNavigationError] = useState<{ key: string; error: string }>();
  const navigation = useRef<AbortController | null>(null);
  useEffect(() => () => navigation.current?.abort(), [key, active]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const saved = track && contributionExtent(widget.options?.tradeExtent, track.branch);
  const first = saved?.[0] ?? (widget.time.mode === 'range' ? window?.firstHour : track?.branch.firstHour) ?? 0;
  const last = saved?.[1] ?? (widget.time.mode === 'range' ? window?.lastHour : track?.branch.lastHour) ?? 0;
  const hour = Math.floor(widget.time.mode === 'follow' || widget.time.mode === 'pin' ? window?.lastHour ?? last : last);
  const marketFilter = marketView ? worldFilter(widget.countries, fallback ? [fallback] : [], groups.data, selectionHour, lastHour) : null;
  const marketTags = marketFilter?.tags ? [...marketFilter.tags].sort() : [];
  // Countries at an hour still being prepared are not known yet, never unavailable.
  const marketReason = marketFilter?.reason ?? (marketView && !all && groups.data && marketFilter?.known && !marketTags.length ? 'Selected countries are unavailable at this time.' : null);
  const marketReady = track && marketView && !marketReason && (all || !!groups.data && marketTags.length > 0)
    && selectionHour >= track.branch.firstHour && selectionHour <= track.branch.lastHour;
  // Playback's newest hour is read once the read in flight lands; each reply is dated.
  const market = useFollowedProduct<MarketLendLeaseReply>(runtime.client, marketReady ? marketRequest(track, selectionHour, marketTags) : null,
    ['hour'], active, validateMarketReply, track?.source);
  const prepared = !!runtime.client.preparedQuery;
  const parity = import.meta.env.MODE === 'parity' ? useParityPreparation() : null;
  const view: { from: number; to: number; width?: number } = parity?.requested ? { from: first, to: last } : viewport?.key === key ? viewport : { from: first, to: last, width: 1 };
  const groupTrade = useMemo(() => groups.data ? decodeTrade({ branchId: groups.data.branchId, decimals: 5, resources: [], countries: [], flows: [], reads: [], known: [], partial: [], warnings: [],
    countryIdentities: groups.data.countryIdentities.map(country => ({ tag: country.tag, runs: country.identities })) }) : null, [groups.data]);
  const groupScope = useMemo(() => groupTrade ? tradeScope(groupTrade, widget.countries, resolved, groups.data, lastHour, widget.options?.between === 'between') : null,
    [groupTrade, widget.countries, resolved, groups.data, lastHour, widget.options?.between]);
  const selection = useMemo(() => groupScope?.kind === 'countries' ? { identities: [...(groupScope.countryIds ?? [])],
    ...(groupScope.membership ? { membership: Object.fromEntries(groupScope.membership), unobserved: groupScope.unobserved ?? [] } : {}), between: groupScope.between }
    : { between: widget.options?.between === 'between' }, [groupScope, widget.options?.between]);
  const pageAt = (at: typeof view) => prepared ? { ...at, basis: first, measure: widget.options?.measure ?? 'current', ...selection,
    hours: [last], ...(parity?.requested ? { requireComplete: true } : {}) } : undefined;
  const page = pageAt(view);
  // One read at a time while the chart's view moves (`useViewRead`).
  const content = useContentRevision(runtime.client);
  const read = useViewRead<typeof view, PreparedTradeReply>(ready ? JSON.stringify([key, first, last, selection, prepared, !!parity?.requested, content]) : null,
    view, prepared ? JSON.stringify([view.from, view.to, view.width]) : '', active,
    (at, signal, publish, restart) => loadProduct(runtime.client, tradeFlowsRequest(track!, tags, pageAt(at)), signal,
      value => validateTradeFlowsReply(value) as PreparedTradeReply, track?.source, restart, publish), runtime.client);
  const product = { ...read, error: useHeldFailure(read.error, read.data !== undefined, read.retry) };
  const scale = useProduct<PreparedTradeReply>(runtime.client, prepared && ready && widget.options?.scale !== 'moment'
    ? tradeFlowsRequest(track, tags, { ...page, from: first, to: last, width: 1, hours: [last] }) : null, active,
  value => validateTradeFlowsReply(value) as PreparedTradeReply, track?.source);
  const pointHour = point?.key === key ? point.hour : null;
  const detailHour = detail?.key === key ? detail.hour : null;
  const exactPage = prepared ? { basis: first, measure: widget.options?.measure ?? 'current', ...selection, from: hour, to: hour, width: 1,
    hours: [hour, last, ...(detailHour === null ? [] : [detailHour])] } : undefined;
  // The hours last read stay merged in while playback's newest hours are read.
  const reached = useFollowedProduct<PreparedTradeReply>(runtime.client, prepared && ready ? tradeFlowsRequest(track, tags, exactPage) : null,
    ['from', 'to', 'hours'], active, value => validateTradeFlowsReply(value) as PreparedTradeReply, track?.source);
  const answered = reached.data && reached.moment ? JSON.parse(reached.moment) as [number, number, number[] | null] : undefined;
  const pointed = useProduct<PreparedTradeReply>(runtime.client, prepared && ready && pointHour !== null ? tradeFlowsRequest(track, tags,
    { ...exactPage, from: pointHour, to: pointHour, hours: [pointHour, last] }) : null, active,
    value => validateTradeFlowsReply(value) as PreparedTradeReply, track?.source);
  const retained = useMemo(() => new NumericReply<PreparedTradeReply>(), [runtime.client]);
  const display = retained.update(JSON.stringify([key, first, last, selection]), product.data, reply => ({ ...reply,
    resources: reply.resources.map(resource => ({ ...resource, name: null, icon: null })), warnings: [] }));
  const data = useMemo(() => {
    if (marketView) return undefined;
    let data = product.data ?? display;
    if (!data) return undefined;
    for (const extra of [reached.data, pointed.data]) if (extra) data = mergePreparedTrade(data, extra);
    return data;
  }, [product.data, display, reached.data, pointed.data, marketView]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => {
    if (view.from > view.to) return;
    setViewport(before => before?.key === key && before.from === view.from && before.to === view.to && before.width === view.width ? before : { key, ...view });
  }, [key]);
  const onHover = useCallback((hour: number | null) => setPoint(before => {
    const at = hour === null ? null : Math.floor(hour);
    return before?.key === key && before.hour === at ? before : { key, hour: at };
  }), [key]);
  const onReachedHour = useCallback((hour: number | null) => setDetail(before => {
    const at = hour === null ? null : Math.floor(hour);
    return before?.key === key && before.hour === at ? before : { key, hour: at };
  }), [key]);
  /** The keyboard's next change among exactly the flows a chart draws (`flows`, by key): read
   * with the widget's selection, or for one country's whole trade with its own `tags`. */
  const navigateFlows = useCallback((flows: readonly string[], country?: readonly string[]): NonNullable<TimeChartProps['onNavigateHour']> => (action, from, x, until) => {
    if (!track || !active) return;
    navigation.current?.abort(); const controller = new AbortController(); navigation.current = controller;
    setNavigationError(undefined);
    void preparedSeriesQuery(runtime.client, track.source, 'trade-flows' as Parameters<typeof preparedSeriesQuery>[2], { branchId: track.branch.id,
      ...(country ? { tags: country, between: false } : { tags, ...selection }), flows,
      op: 'navigate', key: action, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) }, controller.signal)
      .then(value => { if (!controller.signal.aborted) { const hour = (value as { hour: number | null }).hour;
        onHover(hour); setKeyboard(before => ({ key, hour, revision: (before?.revision ?? 0) + 1 })); } })
      .catch(error => { if (!controller.signal.aborted) setNavigationError({ key, error: String(error) }); });
  }, [runtime.client, key, track, tags, selection, active, onHover]);
  const scope = useMemo(() => data ? tradeScope(decodeTrade(data), widget.countries, resolved, groups.data, lastHour,
    widget.options?.between === 'between') : null, [data, widget.countries, widget.options?.between, resolved, groups.data, lastHour]);
  return { track, all, resolved, groups, scope, ...product, data, market, marketReason,
    scaleData: prepared ? scale.data : data,
    scaleError: prepared && scale.data === undefined ? scale.error : undefined, retryScale: scale.retry,
    retry: () => { product.retry(); reached.retry(); pointed.retry(); scale.retry(); },
    hasExactHour: (hour: number) => !prepared || (data?.exactHours ?? []).includes(Math.floor(hour)),
    error: product.error ?? reached.error ?? pointed.error ?? (navigationError?.key === key ? navigationError.error : undefined),
    onSummaryView: prepared ? onSummaryView : undefined, onHover: prepared ? onHover : undefined,
    onReachedHour: prepared ? onReachedHour : undefined,
    navigateFlows: prepared ? navigateFlows : undefined, keyboardHour: keyboard?.key === key ? keyboard.hour : undefined,
    keyboardRevision: keyboard?.key === key ? keyboard.revision : undefined, pointHour,
    reachedHour: answered?.[0], reachedDetail: answered?.[2]?.[2] as number | undefined, page: page ? { ...page, hours: [hour, last, ...(pointHour === null ? [] : [pointHour])] } : undefined,
    parityPrepare: parity?.prepare, parityComplete: !!product.data && (!prepared || product.data.coverage?.complete === true) && !!parity?.requested };

}

/** The Resources setting's choices: the campaign's own resources, named and pictured
 * as the game does, one per token (a repeated declaration is chosen with its token). */
export function useTradeResourceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useTradeFlows(widget, runtime, true);
  return { resources: (data?.resources ?? []).filter(item => item.ordinal === 1).map(item => ({ value: item.token, label: resourceName(item), icon: item.icon })) };
}
