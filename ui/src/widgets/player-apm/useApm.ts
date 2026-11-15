import { useMemo } from 'react';
import type { OptionChoices, WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { selectedTags } from '../../dashboard/countrySelection';
import { resolveCountries } from '../../dashboard/groups';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useProduct, useFollowedProduct } from '../../dashboard/products';
import { useFallbackHour } from '../../dashboard/follow/useFollow.ts';
import type { ApmParams, ApmReply } from '../../generated/dashboard';
import { GRAPH_OPTIONS, TABLE_APM_OPTIONS } from './definition';
import { playerNames, validateApm } from './apmData';

export function useApmRegistry(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'player-apm',
    params: { branchId: track.branch.id, group: 'player', registry: true, firstHour: null, lastHour: null, tags: null, membership: null, players: [], buckets: 1 } } : null;
  return useProduct<ApmReply>(runtime.client, request, active, validateApm, track?.source);
}

export function useApmChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useApmRegistry(widget, runtime, true);
  return useMemo(() => {
    const players = data?.players ?? [];
    const names = playerNames(players);
    return { players: players.map(player => ({ value: setToken(player.id), label: `${names.get(player.id)} · ${player.tags.join(', ')}` })) };
  }, [data]);
}

/** The widget's eAPM by request: by the fallback's clock while following unless `followed` (its values at the head are
 * then pushed, and nothing is asked here), else for its pinned hour or range. `head` names what a followed head asks. */
export function useApm({ widget, runtime, active }: WidgetProps, graph: boolean, view?: { identity: string; from: number; to: number; width: number } | null,
  followed = false) {
  const playbackHour = useFallbackHour(runtime.clock, active && widget.time.mode === 'follow' && !followed);
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const identity = JSON.stringify([track?.source.id, track?.branch.id, widget.time]);
  const options = resolveOptions(graph ? GRAPH_OPTIONS : TABLE_APM_OPTIONS, widget.options);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const rangeFirst = window && Math.max(track!.branch.firstHour, Math.floor(window.firstHour));
  const rangeLast = window && Math.min(track!.branch.lastHour, Math.floor(window.lastHour));
  const first = graph && widget.time.mode === 'follow' ? track?.branch.firstHour : rangeFirst;
  const last = graph && widget.time.mode === 'follow' ? track?.branch.lastHour : rangeLast;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const selected = selectedTags(widget.countries, track);
  const resolved = useMemo(() => widget.countries.kind === 'all' ? null
    : resolveCountries(widget.countries, selected, groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
  // eslint-disable-next-line react-hooks/exhaustive-deps
  [widget.countries, JSON.stringify(selected), groups.data, track?.branch.lastHour]);
  const boundedView = graph && view?.identity === identity && first != null && last != null && view.from <= last && view.to >= first ? view : null;
  const firstHour = first == null || rangeFirst == null ? null : Math.max(first, boundedView ? Math.floor(boundedView.from) : rangeFirst);
  const lastHour = last == null || rangeLast == null ? null : Math.min(last, boundedView ? Math.floor(boundedView.to) : rangeLast);
  const known = firstHour !== null && lastHour !== null && (resolved?.known(firstHour, lastHour) ?? true);
  const membershipGap = resolved?.unobserved.some(span => firstHour !== null && lastHour !== null && span.firstHour <= lastHour && firstHour <= span.lastHour);
  const reason = resolved?.reason ?? (membershipGap ? 'Country group membership was not recorded throughout this period.' : null);
  const params: ApmParams | null = !followed && track && firstHour !== null && lastHour !== null && firstHour <= lastHour && known && !reason ? {
    branchId: track.branch.id, group: options.group === 'player' ? 'player' : 'country', registry: false, firstHour, lastHour,
    tags: resolved?.tags ?? null, players: contentSet(options.players).map(contentKey),
    membership: resolved?.membership ? [...resolved.membership.values()].flat() : null,
    buckets: graph ? Math.max(1, Math.min(4096, Math.ceil(view?.width ?? 1024))) : 1,
  } : null;
  const product = useFollowedProduct<ApmReply>(runtime.client, track && params ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'player-apm', params } : null, ['firstHour', 'lastHour', 'buckets'], active, validateApm, track?.source);
  // The hours the shown reply answers; while the newest are read it stays shown, named for its own.
  const answered = useMemo(() => {
    if (!product.moment) return null;
    const [firstHour, lastHour] = JSON.parse(product.moment) as [number, number, number];
    return { firstHour, lastHour };
  }, [product.moment]);
  // A followed head names the countries and players asked, not the hours: the service takes the trailing window.
  // Group membership the recording leaves unobserved anywhere keeps the request path, which refuses such windows.
  const head = track && (!resolved || (!resolved.reason && !resolved.unobserved.length)) ? {
    branchId: track.branch.id, group: options.group === 'player' ? 'player' as const : 'country' as const, tags: resolved?.tags ?? null,
    membership: resolved?.membership ? [...resolved.membership.values()].flat() : null, players: contentSet(options.players).map(contentKey),
  } : null;
  return { track, identity, options, window, first, last, playbackHour, known, reason, groups, answered, head, ...product };
}
