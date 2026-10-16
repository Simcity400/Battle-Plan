import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartSummary } from '../../charts/chartSummary';
import { mergeReachedRows, preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { withoutHours } from '../../charts/timeline/runState';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { CountrySelection, WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct, useProducts } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import { groupNumbers } from '../../dashboard/groupsReply';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { BoundTrack } from '../../dashboard/model';
import { resolveCountries } from '../../dashboard/groups';
import type { CountryGroupReply, ResourceBalanceReply } from '../../generated/dashboard';
import { resourceName, resourceNumbers, validateResourceBalanceReply } from './balanceData';
import { resourceCountryData, resourceCountrySpans } from './resourceDrill';
import type { ResourceCountrySelection } from './resourceDrill';

export type PreparedResourceReply = ResourceBalanceReply & {
  summaries?: { id: number; supply: ChartSummary; demand: ChartSummary; parts: ChartSummary[]; total: ChartSummary }[];
  campaignSizes?: Record<string, string>;
  exactHours?: number[];
  coverage?: { firstHour: number; lastHour: number; complete: boolean } | null;
};

/** Reached facts join the same inventory while accepted viewport geometry stays. */
export function mergePreparedResource(a: PreparedResourceReply, b: PreparedResourceReply): PreparedResourceReply {
  if (a.branchId !== b.branchId || a.tag !== b.tag) throw new Error('Resource pages belong to different countries.');
  const first = new Map(a.series.map(row => [row.id, row])), second = new Map(b.series.map(row => [row.id, row]));
  const ids = [...new Set([...first.keys(), ...second.keys()])].sort((a, b) => a - b);
  const rows = ids.map(id => { const old = first.get(id) ?? { id, runs: [] }, next = second.get(id) ?? { id, runs: [] };
    return { ...old, runs: withoutHours(old.runs, next.runs) }; });
  return { ...a, resources: [...new Map([...a.resources, ...b.resources].map(resource => [resource.id, resource])).values()].sort((a, b) => a.id - b.id),
    series: mergeReachedRows(rows, ids.map(id => second.get(id) ?? { id, runs: [] }), row => String(row.id)),
    dealHours: mergeReachedRows([{ runs: a.dealHours }], [{ runs: b.dealHours }], () => '')[0].runs,
    trades: [...new Map([...a.trades, ...b.trades].map(row => [JSON.stringify(row), row])).values()].sort((a, b) => a.firstHour - b.firstHour),
    exactHours: [...new Set([...(a.exactHours ?? []), ...(b.exactHours ?? [])])] };
}

/** Display geometry and exact moments share the same prepared adapter. */
/** The widget's country and its resource balance reply. */
export function useResourceBalance(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, within?: readonly { firstHour: number; lastHour: number }[] | null) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const key = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag]);
  const [viewport, setViewport] = useState<{ key: string; from: number; to: number; width: number; resources: readonly number[] }>();
  const [hover, setHover] = useState<{ key: string; hour: number | null }>();
  const [keyboard, setKeyboard] = useState<{ key: string; hour: number | null; revision: number }>();
  const [navigationError, setNavigationError] = useState<{ key: string; error: string }>();
  const navigation = useRef<AbortController | null>(null);
  useEffect(() => () => navigation.current?.abort(), [key, active]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const prepared = !!runtime.client.preparedQuery;
  const parity = import.meta.env.MODE === 'parity' ? useParityPreparation() : null;
  const view = parity?.requested && track ? { from: track.branch.firstHour, to: track.branch.lastHour }
    : viewport?.key === key ? viewport : widget.options?.view === 'time' && track
      ? { from: window?.firstHour ?? track.branch.firstHour, to: hour, width: 1 } : { from: hour, to: hour };
  const exact = [hour, ...(hover?.key === key && hover.hour !== null ? [hover.hour] : [])];
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'resource-balance',
    params: { branchId: track.branch.id, tag, ...(prepared ? { ...view, hours: [Math.floor(view.to)], within, ...(parity?.requested ? { requireComplete: true } : {}) } : {}) } } : null;
  const product = useProduct<PreparedResourceReply>(runtime.client, request, active,
    value => validateResourceBalanceReply(value) as PreparedResourceReply, track?.source);
  const points = useProduct<PreparedResourceReply>(runtime.client, track && tag && prepared ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'resource-balance',
    params: { branchId: track.branch.id, tag, from: hour, to: hour, hours: exact, within } } : null, active,
  value => validateResourceBalanceReply(value) as PreparedResourceReply, track?.source);
  const onSummaryView = useCallback((view: { from: number; to: number; width: number }, resources: readonly number[] = []) => {
    if (view.from > view.to) return;
    setViewport(before => before?.key === key && before.from === view.from && before.to === view.to && before.width === view.width && JSON.stringify(before.resources) === JSON.stringify(resources)
      ? before : { key, ...view, resources });
  }, [key]);
  const onHover = useCallback((hour: number | null) => setHover(before => {
    const at = hour === null ? null : Math.floor(hour);
    return before?.key === key && before.hour === at ? before : { key, hour: at };
  }), [key]);
  const onNavigateHour = useCallback((resources: readonly number[], action: Parameters<NonNullable<TimeChartProps['onNavigateHour']>>[0],
    from: number | null, x: readonly [number, number], until: number | null) => {
    if (!track || !active) return;
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    setNavigationError(undefined);
    void preparedSeriesQuery(runtime.client, track.source, 'resource-balance', { branchId: track.branch.id, tag, resources,
      op: 'navigate', within, key: action, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) }, controller.signal)
      .then(value => { if (!controller.signal.aborted) { const hour = (value as { hour: number | null }).hour;
        onHover(hour); setKeyboard(before => ({ key, hour, revision: (before?.revision ?? 0) + 1 })); } })
      .catch(error => { if (!controller.signal.aborted) setNavigationError({ key, error: String(error) }); });
  }, [runtime.client, track, key, active, tag, within, onHover]);
  const retained = useMemo(() => new NumericReply<PreparedResourceReply>(), [runtime.client]);
  const numbers = retained.update(JSON.stringify([key, within]), product.data, resourceNumbers);
  const data = useMemo(() => {
    const base = product.data ?? numbers;
    return base && points.data ? mergePreparedResource(base, points.data) : base;
  }, [product.data, numbers, points.data]);
  return { track, tag, ...product, numbers, data, retry: () => { product.retry(); points.retry(); },
    hasExactHour: (hour: number) => !prepared || (data?.exactHours ?? []).includes(Math.floor(hour)),
    error: product.error ?? points.error ?? (navigationError?.key === key ? navigationError.error : undefined),
    onSummaryView: prepared ? onSummaryView : undefined, onHover: prepared ? onHover : undefined,
    onNavigateHour: prepared ? onNavigateHour : undefined,
    keyboardHour: keyboard?.key === key ? keyboard.hour : undefined, keyboardRevision: keyboard?.key === key ? keyboard.revision : undefined,
    parityComplete: !!product.data && (!prepared || product.data.coverage?.complete === true) && !!parity?.requested, parityPrepare: parity?.prepare,
    view: prepared ? { ...view, hours: exact, within } : undefined,
    contentPending: !!numbers && !product.data && !product.error };
}

/** Only the dated holder's aliases supply a selected breakdown. */
export function useResourceCountry(selection: ResourceCountrySelection | null, runtime: WidgetRuntime, track: BoundTrack | undefined,
  active: boolean, group?: CountrySelection | null, view?: { from: number; to: number; width?: number; hours: number[] }, moment?: number | null) {
  const catalogue = useCountryGroups(runtime, track, active && !!selection);
  const identities = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const history = identities.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]), catalogue.data, groupNumbers);
  const members = useMemo(() => group?.kind === 'group'
    ? resolveCountries(group, [], history, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER) : null,
  [group, history, track?.branch.lastHour]);
  const spans = useMemo(() => selection && history && track && history.branchId === track.branch.id
    && selection.campaign === track.source.metadata.campaignKey && selection.branch === track.branch.id
    ? resourceCountrySpans(history.countryIdentities, selection, track.branch.firstHour, track.branch.lastHour, members?.reason ? new Map() : members?.membership) : null,
  [selection, history, track?.source.metadata.campaignKey, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour, members]);
  const requests = track && spans ? [...spans.keys()].map(tag => ({ sourceId: track.source.id, revision: track.source.revision,
    product: 'resource-balance', params: { branchId: track.branch.id, tag, ...(runtime.client.preparedQuery ? { ...view,
      within: spans.get(tag)?.map(({ firstHour, lastHour }) => ({ firstHour, lastHour })), hours: [Math.floor(view?.to ?? track.branch.lastHour)] } : {}) } })) : [];
  const validate = (value: unknown) => {
    const reply = validateResourceBalanceReply(value);
    if (!track || reply.branchId !== track.branch.id || !spans?.has(reply.tag)) throw new Error('The selected resource history belongs to another country or branch.');
    return reply as PreparedResourceReply;
  };
  const product = useProducts<PreparedResourceReply>(runtime.client, requests, active && !!selection, validate, track?.source);
  const hours = [...new Set([...(view?.hours ?? []), ...(moment === null || moment === undefined ? [] : [moment])])];
  const points = useProducts<PreparedResourceReply>(runtime.client, runtime.client.preparedQuery && hours.length ? requests.map(request => ({ ...request,
    params: { ...request.params, from: hours[0], to: hours[0], width: undefined, hours } })) : [], active && !!selection, validate, track?.source);
  const store = useMemo(() => new NumericReply<PreparedResourceReply[]>(), [runtime.client]);
  const numbers = store.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, selection, [...(spans ?? [])]]), product.data, replies => replies.map(resourceNumbers));
  const data = useMemo(() => {
    const base = product.data ?? numbers;
    return spans && selection && base ? resourceCountryData(points.data?.length === base.length
      ? base.map((reply, index) => mergePreparedResource(reply, points.data![index])) : base, spans, selection.tag) : undefined;
  }, [product.data, numbers, points.data, spans, selection]);
  const reason = catalogue.error ?? product.error ?? members?.reason ?? (!history ? 'Loading the selected country identity…'
    : !spans ? 'The selected country identity is unknown or conflicting at its original choice.'
      : !data ? 'Loading the selected resource history…' : null);
  return { data, numbers, spans, reason, error: catalogue.error ?? product.error ?? points.error,
    retry: () => { catalogue.retry(); product.retry(); points.retry(); } };
}

/** The Resources setting's choices: the campaign's own resources, named and pictured
 * as the game does, one per token (a repeated declaration is chosen with its token). */
export function useResourceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useResourceBalance(widget, runtime, true);
  return { resources: (data?.resources ?? []).filter(item => item.ordinal === 1).map(item => ({ value: item.token, label: resourceName(item), icon: item.icon })),
    partResource: (data?.resources ?? []).map(item => ({ value: setToken(JSON.stringify([item.token, item.ordinal])), label: resourceName(item), icon: item.icon })) };
}
